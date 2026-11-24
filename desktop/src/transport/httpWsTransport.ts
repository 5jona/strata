import type { ContainerJobProgressPayload } from '../types/protocol.ts';
import {
  type StrataDesktopEventMap,
  type StrataDesktopEventName,
  type StrataDesktopMethod,
  type StrataDesktopRpcMap,
  type StrataEventEnvelope,
  StrataRpcError,
} from '../types/rpc.ts';
import {
  type DecodedServerPtyFrame,
  decodeServerPtyFrame,
  encodePtyAckFrame,
  encodePtyDataFrame,
  encodePtyResizeFrame,
  type PtySessionMeta,
  parsePtySessionMeta,
} from './ptyProtocol.ts';

export interface ImportUploadOptions {
  containerId: string;
  file: File;
  sizeMb: number;
  hostname?: string;
  sshPort?: number;
  autoStart: boolean;
  hostBridge: boolean;
  noatime: boolean;
  onProgress?: (loaded: number, total: number) => void;
  signal?: AbortSignal;
}

export interface PtyConnectionHandlers {
  onOpen?: () => void;
  onMeta?: (meta: PtySessionMeta) => void;
  onData: (chunk: Uint8Array) => void;
  onExit?: (exitCode: number, signal?: number) => void;
  onClose?: (reason: string) => void;
  onError?: (message: string) => void;
}

export interface PtySessionHandle {
  sendInput(data: string | Uint8Array): void;
  sendResize(cols: number, rows: number): void;
  sendAck(bytes: number): void;
  close(): void;
}

export type EventListener = (envelope: StrataEventEnvelope) => void;

export interface StrataTransport {
  readonly mode: 'http-ws' | 'mock';
  readonly baseUrl: string;
  invoke<M extends StrataDesktopMethod>(
    method: M,
    params?: StrataDesktopRpcMap[M]['params'],
  ): Promise<StrataDesktopRpcMap[M]['result']>;
  subscribe(listener: EventListener): () => void;
  openPty(
    containerId: string,
    cols: number,
    rows: number,
    handlers: PtyConnectionHandlers,
    resumeSessionId?: string,
  ): PtySessionHandle;
  killPty(sessionId: string): Promise<void>;
  uploadImage(options: ImportUploadOptions): Promise<ContainerJobProgressPayload>;
  downloadExport(containerId: string, format: 'tar.gz' | 'img.gz'): Promise<void>;
  dispose(): void;
}

interface JsonRpcResponse<T> {
  jsonrpc: '2.0';
  id: number | string;
  result?: T;
  error?: { code: number; message: string };
}

/**
 * Pure Bearer-token HTTP + WebSocket transport communicating with `stratad`.
 */
export class HttpWsTransport implements StrataTransport {
  readonly mode = 'http-ws' as const;
  readonly baseUrl: string;
  private readonly token: string;
  private readonly fetchImpl: typeof fetch;
  private rpcSeq = 1;
  private listeners = new Set<EventListener>();
  private eventWs: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempt = 0;
  private disposed = false;

  constructor(baseUrl: string, token: string, fetchImpl?: typeof fetch) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    this.token = token.trim();
    // Always invoke fetch with `globalThis` receiver; Chromium/WebView2 throws
    // `Failed to execute 'fetch' on 'Window': Illegal invocation` if called as `this.fetchImpl(...)`.
    this.fetchImpl =
      !fetchImpl || fetchImpl === globalThis.fetch
        ? (input, init) => globalThis.fetch(input, init)
        : (input, init) => fetchImpl.call(globalThis, input, init);
  }

  private authHeaders(): Record<string, string> {
    return this.token ? { Authorization: `Bearer ${this.token}` } : {};
  }

  private wsSubprotocols(): string[] {
    return this.token ? [`strata-auth.${this.token}`] : [];
  }

  private toWsUrl(path: string): string {
    const wsBase = this.baseUrl.replace(/^http:/i, 'ws:').replace(/^https:/i, 'wss:');
    return `${wsBase}${path}`;
  }

  async invoke<M extends StrataDesktopMethod>(
    method: M,
    params?: StrataDesktopRpcMap[M]['params'],
  ): Promise<StrataDesktopRpcMap[M]['result']> {
    const id = String(this.rpcSeq++);
    const response = await this.fetchImpl(`${this.baseUrl}/api/rpc`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...this.authHeaders(),
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id,
        method,
        params: params === undefined ? null : params,
      }),
    });

    if (!response.ok) {
      let detail = `HTTP ${response.status}`;
      try {
        const raw = (await response.text()).trim();
        if (raw) {
          try {
            const errJson = JSON.parse(raw) as { error?: string };
            detail = errJson.error
              ? `${detail}: ${errJson.error}`
              : `${detail}: ${raw.slice(0, 200)}`;
          } catch {
            detail = `${detail}: ${raw.slice(0, 200)}`;
          }
        }
      } catch {
        // Ignore unreadable body
      }
      throw new StrataRpcError(-32000, detail);
    }

    const body = (await response.json()) as JsonRpcResponse<StrataDesktopRpcMap[M]['result']>;
    if (body.error) {
      throw new StrataRpcError(body.error.code, body.error.message);
    }
    return body.result as StrataDesktopRpcMap[M]['result'];
  }

  subscribe(listener: EventListener): () => void {
    this.listeners.add(listener);
    if (!this.eventWs && !this.disposed) {
      this.connectEventsSocket();
    }
    return () => {
      this.listeners.delete(listener);
      if (this.listeners.size === 0) {
        this.closeEventsSocket();
      }
    };
  }

  private connectEventsSocket(): void {
    if (this.disposed || typeof WebSocket === 'undefined') return;
    try {
      const ws = new WebSocket(this.toWsUrl('/ws/events'), this.wsSubprotocols());
      this.eventWs = ws;
      ws.onopen = () => {
        this.reconnectAttempt = 0;
      };
      ws.onmessage = (ev) => {
        if (typeof ev.data !== 'string') return;
        try {
          const parsed = JSON.parse(ev.data) as {
            event?: StrataDesktopEventName;
            payload?: StrataDesktopEventMap[StrataDesktopEventName];
          };
          if (parsed.event && parsed.payload !== undefined) {
            const envelope = {
              event: parsed.event,
              payload: parsed.payload,
            } as StrataEventEnvelope;
            for (const fn of this.listeners) {
              fn(envelope);
            }
          }
        } catch {
          // Ignore malformed event envelope
        }
      };
      ws.onclose = () => {
        this.eventWs = null;
        if (!this.disposed && this.listeners.size > 0) {
          const delay = Math.min(8000, 600 * 2 ** Math.min(this.reconnectAttempt, 4));
          this.reconnectAttempt += 1;
          this.reconnectTimer = setTimeout(() => this.connectEventsSocket(), delay);
        }
      };
      ws.onerror = () => {
        ws.close();
      };
    } catch {
      // Retry on next timer tick if constructor fails
    }
  }

  private closeEventsSocket(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.eventWs) {
      this.eventWs.onclose = null;
      this.eventWs.close();
      this.eventWs = null;
    }
  }

  openPty(
    containerId: string,
    cols: number,
    rows: number,
    handlers: PtyConnectionHandlers,
    resumeSessionId?: string,
  ): PtySessionHandle {
    const query = new URLSearchParams({
      cols: String(Math.max(2, Math.floor(cols))),
      rows: String(Math.max(2, Math.floor(rows))),
    });
    if (resumeSessionId) {
      query.set('session', resumeSessionId);
    }
    const url = this.toWsUrl(`/ws/pty/${encodeURIComponent(containerId)}?${query.toString()}`);
    const ws = new WebSocket(url, this.wsSubprotocols());
    ws.binaryType = 'arraybuffer';

    ws.onopen = () => handlers.onOpen?.();
    ws.onmessage = (ev) => {
      if (typeof ev.data === 'string') {
        const meta = parsePtySessionMeta(ev.data);
        if (meta) handlers.onMeta?.(meta);
        return;
      }
      if (ev.data instanceof ArrayBuffer) {
        const frame: DecodedServerPtyFrame = decodeServerPtyFrame(ev.data);
        if (frame.type === 'data') {
          handlers.onData(frame.payload);
        } else if (frame.type === 'exit') {
          handlers.onExit?.(frame.exitCode, frame.signal);
        }
      }
    };
    ws.onerror = () => handlers.onError?.('PTY WebSocket 通信异常');
    ws.onclose = (ev) => handlers.onClose?.(ev.reason || `连接关闭 (${ev.code})`);

    return {
      sendInput(data) {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(encodePtyDataFrame(data));
        }
      },
      sendResize(nextCols, nextRows) {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(encodePtyResizeFrame(nextCols, nextRows));
        }
      },
      sendAck(bytes) {
        if (ws.readyState === WebSocket.OPEN && bytes > 0) {
          ws.send(encodePtyAckFrame(bytes));
        }
      },
      close() {
        ws.close(1000, 'client closed');
      },
    };
  }

  async killPty(sessionId: string): Promise<void> {
    await this.fetchImpl(`${this.baseUrl}/api/pty/${encodeURIComponent(sessionId)}`, {
      method: 'DELETE',
      headers: this.authHeaders(),
    });
  }

  uploadImage(options: ImportUploadOptions): Promise<ContainerJobProgressPayload> {
    return new Promise((resolve, reject) => {
      const qs = new URLSearchParams({
        id: options.containerId,
        filename: options.file.name,
        sizeMb: String(options.sizeMb),
        autoStart: String(options.autoStart),
        hostBridge: String(options.hostBridge),
        noatime: String(options.noatime),
      });
      if (options.hostname) qs.set('hostname', options.hostname);
      if (options.sshPort) qs.set('sshPort', String(options.sshPort));

      const xhr = new XMLHttpRequest();
      xhr.open('POST', `${this.baseUrl}/api/images/import?${qs.toString()}`);
      if (this.token) {
        xhr.setRequestHeader('Authorization', `Bearer ${this.token}`);
      }
      xhr.setRequestHeader('Content-Type', 'application/octet-stream');

      xhr.upload.onprogress = (ev) => {
        if (ev.lengthComputable) {
          options.onProgress?.(ev.loaded, ev.total);
        }
      };

      const onAbort = () => xhr.abort();
      options.signal?.addEventListener('abort', onAbort, { once: true });

      xhr.onload = () => {
        options.signal?.removeEventListener('abort', onAbort);
        if (xhr.status >= 200 && xhr.status < 300) {
          try {
            resolve(JSON.parse(xhr.responseText) as ContainerJobProgressPayload);
          } catch {
            reject(new Error('导入响应 JSON 解析失败'));
          }
        } else {
          reject(new Error(`导入失败 (HTTP ${xhr.status}): ${xhr.responseText}`));
        }
      };
      xhr.onerror = () => {
        options.signal?.removeEventListener('abort', onAbort);
        reject(new Error('镜像上传网络中断'));
      };
      xhr.onabort = () => {
        reject(new Error('用户取消上传'));
      };
      xhr.send(options.file);
    });
  }

  async downloadExport(containerId: string, format: 'tar.gz' | 'img.gz'): Promise<void> {
    const url = `${this.baseUrl}/api/containers/${encodeURIComponent(containerId)}/export?format=${format}`;
    const resp = await this.fetchImpl(url, { headers: this.authHeaders() });
    if (!resp.ok) {
      throw new Error(`导出失败 (HTTP ${resp.status})`);
    }
    const blob = await resp.blob();
    const objectUrl = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = objectUrl;
    link.download = `strata-${containerId}.${format}`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(objectUrl), 10_000);
  }

  dispose(): void {
    this.disposed = true;
    this.listeners.clear();
    this.closeEventsSocket();
  }
}
