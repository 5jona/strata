import type { ContainerJobProgressPayload } from '../types/protocol.ts';
import type {
  RpcInvokeArgs,
  StrataDesktopMethod,
  StrataDesktopRpcMap,
  StrataEventEnvelope,
} from '../types/rpc.ts';
import type { ImportUploadOptions } from './httpTransfer.ts';
import type { PtySessionMeta } from './ptyProtocol.ts';

export type { ImportUploadOptions };

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
    ...args: RpcInvokeArgs<StrataDesktopRpcMap[M]['params']>
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
