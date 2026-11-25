import {
  type ContainerJobProgressPayload,
  STRATA_PATHS,
  STRATA_PROTOCOL,
} from '../types/protocol.ts';
import {
  type RpcInvokeArgs,
  STRATA_METHODS,
  type StrataDesktopEventMap,
  type StrataDesktopEventName,
  type StrataDesktopMethod,
  type StrataDesktopRpcMap,
  type StrataEventEnvelope,
  StrataRpcError,
} from '../types/rpc.ts';
import type {
  EventListener,
  ImportUploadOptions,
  PtyConnectionHandlers,
  PtySessionHandle,
  StrataTransport,
} from './httpWsTransport.ts';
import { MOCK_DISTRO_CATALOG } from './mockCatalogData.ts';
import { dispatchMockContainerAndServiceRpc } from './mockCoreDispatch.ts';
import { createMockDatasetForDevice, type MockDeviceDataset } from './mockData.ts';
import { type MockRuntimeContext, registerNewMockContainer } from './mockHandlers.ts';
import { createMockPtySession } from './mockPty.ts';
import { handleMockTaskRpc, type MockTaskRecord } from './mockTaskHandlers.ts';
import { handleMockToolsRpc } from './mockToolsHandlers.ts';

/**
 * Full stateful in-memory Strata transport for browser preview and unit tests.
 */
export class MockStrataTransport implements StrataTransport {
  readonly mode = 'mock' as const;
  readonly baseUrl: string;
  private readonly dataset: MockDeviceDataset;
  private readonly listeners = new Set<EventListener>();
  private readonly jobs = new Map<string, ContainerJobProgressPayload>();
  private readonly tasks = new Map<string, MockTaskRecord>();
  private readonly timers = new Set<ReturnType<typeof setTimeout>>();

  constructor(deviceId = 'mock-tablet') {
    this.baseUrl = `mock://${deviceId}`;
    this.dataset = createMockDatasetForDevice(deviceId);
  }

  private ctx(): MockRuntimeContext {
    return {
      dataset: this.dataset,
      jobs: this.jobs,
      emit: (event, payload) => this.emit(event, payload),
      schedule: (delay, fn) => this.schedule(delay, fn),
    };
  }

  private emit<E extends StrataDesktopEventName>(
    event: E,
    payload: StrataDesktopEventMap[E],
  ): void {
    const envelope = { event, payload } as StrataEventEnvelope;
    for (const listener of this.listeners) listener(envelope);
  }

  private schedule(delayMs: number, fn: () => void): void {
    const t = setTimeout(() => {
      this.timers.delete(t);
      fn();
    }, delayMs);
    this.timers.add(t);
  }

  async invoke<M extends StrataDesktopMethod>(
    method: M,
    ...args: RpcInvokeArgs<StrataDesktopRpcMap[M]['params']>
  ): Promise<StrataDesktopRpcMap[M]['result']> {
    const res = this.dispatch(method, args[0]);
    return structuredClone(res) as StrataDesktopRpcMap[M]['result'];
  }

  private dispatch(method: StrataDesktopMethod, rawParams: unknown): unknown {
    const p = (rawParams ?? {}) as Record<string, unknown>;
    const ctx = this.ctx();

    if (method.startsWith('tools.')) return handleMockToolsRpc(ctx, this.tasks, method, rawParams);
    if (method.startsWith('task.')) return handleMockTaskRpc(ctx, this.tasks, method, rawParams);
    if (method.startsWith('container.') || method.startsWith('service.')) {
      return dispatchMockContainerAndServiceRpc(ctx, method, p);
    }

    switch (method) {
      case STRATA_METHODS.DAEMON_PING:
        return { pong: true, pid: 1420, uptimeMs: 18_400_000 };
      case STRATA_METHODS.DAEMON_STATUS: {
        const list = Object.values(this.dataset.containers);
        return {
          pid: 1420,
          uid: 0,
          uptimeMs: 18_400_000,
          defaultContainer: list.find((c) => c.snapshot.isDefault)?.snapshot.id ?? 'main',
          totalContainers: list.length,
          runningContainers: list.filter((c) => c.snapshot.phase === 'running').length,
          activeTasks: Array.from(this.tasks.values()).filter((t) => t.summary.running).length,
          sideveilConnected: true,
        };
      }
      case STRATA_METHODS.DAEMON_HEALTH:
        return {
          ok: true,
          daemon: {
            pid: 1420,
            uid: 0,
            uptimeMs: 18_400_000,
            defaultContainer: 'main',
            totalContainers: 2,
            runningContainers: 1,
            activeTasks: 0,
            sideveilConnected: true,
          },
          containers: Object.values(this.dataset.containers).map((c) => c.snapshot),
          capabilities: this.dataset.capabilities,
        };
      case STRATA_METHODS.DAEMON_CAPABILITIES:
        return this.dataset.capabilities;
      case STRATA_METHODS.DAEMON_LOGS:
        return { logPath: STRATA_PATHS.DEFAULT_DAEMON_LOG_PATH, lines: this.dataset.daemonLogs };
      case STRATA_METHODS.DAEMON_RELOAD_CONFIG:
        return {
          version: 1,
          defaultContainer: 'main',
          sideveil: {
            enabled: false,
            hubSock: STRATA_PATHS.DEFAULT_SIDEVEIL_HUB_SOCK,
            pluginId: STRATA_PROTOCOL.SIDEVEIL_PLUGIN_ID,
          },
          web: {
            enabled: this.dataset.webStatus.enabled,
            lanEnabled: this.dataset.webStatus.lanEnabled,
            port: this.dataset.webStatus.port,
          },
          containers: Object.fromEntries(
            Object.entries(this.dataset.containers).map(([k, v]) => [k, v.config]),
          ),
        };
      case STRATA_METHODS.WEB_STATUS:
        return this.dataset.webStatus;
      case STRATA_METHODS.WEB_CONFIGURE: {
        if (typeof p.enabled === 'boolean') this.dataset.webStatus.enabled = p.enabled;
        if (typeof p.lanEnabled === 'boolean') {
          this.dataset.webStatus.lanEnabled = p.lanEnabled;
          this.dataset.webStatus.listenAddr = p.lanEnabled ? '0.0.0.0' : '127.0.0.1';
        }
        if (typeof p.port === 'number') this.dataset.webStatus.port = p.port;
        return this.dataset.webStatus;
      }
      case STRATA_METHODS.WEB_ROTATE_TOKEN:
        return {
          token: `mock_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`,
          tokenPath: this.dataset.webStatus.tokenPath,
          sessionsRevoked: 1,
        };
      case STRATA_METHODS.DISTRO_CATALOG:
        return {
          mirror: (p.mirror as 'tuna' | 'official' | 'custom') ?? 'tuna',
          baseUrl: STRATA_PROTOCOL.DEFAULT_DISTRO_MIRROR_TUNA,
          cached: false,
          fetchedAtMs: Date.now(),
          entries: MOCK_DISTRO_CATALOG,
        };
      case STRATA_METHODS.EVENTS_SUBSCRIBE:
        return { subscribed: true };
      case STRATA_METHODS.HUD_STATE:
      case STRATA_METHODS.HUD_SET:
        return { enabled: false, clickThrough: false, isDark: true, sideveilConnected: false };
      case STRATA_METHODS.SESSION_RELEASE:
        return { released: true };
      case STRATA_METHODS.SESSION_ACQUIRE:
        throw new StrataRpcError(-32601, 'mock mode does not acquire host CLI attach leases');
    }
  }

  subscribe(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  openPty(
    containerId: string,
    _cols: number,
    _rows: number,
    handlers: PtyConnectionHandlers,
    resumeSessionId?: string,
  ): PtySessionHandle {
    const hostname = this.dataset.containers[containerId]?.snapshot.hostname ?? containerId;
    return createMockPtySession(containerId, hostname, handlers, resumeSessionId);
  }

  async killPty(_sessionId: string): Promise<void> {}

  async uploadImage(options: ImportUploadOptions): Promise<ContainerJobProgressPayload> {
    const total = Math.max(1, options.file.size);
    for (const ratio of [0.25, 0.6, 1]) {
      if (options.signal?.aborted) throw new Error('用户取消上传');
      options.onProgress?.(Math.round(total * ratio), total);
      await new Promise((r) => setTimeout(r, 120));
    }
    registerNewMockContainer(
      this.ctx(),
      options.containerId,
      options.sizeMb,
      options.autoStart,
      options.sshPort,
    );
    const now = Date.now();
    return {
      jobId: `import-${now.toString(36)}`,
      container: options.containerId,
      kind: 'import',
      state: 'completed',
      stage: 'done',
      bytesDone: total,
      bytesTotal: total,
      percent: 100,
      message: '本地镜像导入完成',
      startedAtMs: now,
      updatedAtMs: now,
    };
  }

  async downloadExport(_containerId: string, _format: 'tar.gz' | 'img.gz'): Promise<void> {}

  dispose(): void {
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    this.listeners.clear();
  }
}
