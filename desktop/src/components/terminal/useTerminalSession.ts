import type { Terminal } from '@xterm/xterm';
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import type { PtySessionHandle, StrataTransport } from '../../transport/httpWsTransport.ts';
import { PtyWatermarkController } from '../../transport/ptyProtocol.ts';
import type { TerminalShellId } from './terminalPrefs.ts';

export type TerminalConnectionStatus = 'connecting' | 'connected' | 'reconnecting' | 'disconnected';

export interface TerminalSessionState {
  status: TerminalConnectionStatus;
  disconnectReason: string | null;
  reconnectAttempt: number;
  maxReconnectAttempts: number;
  activeShell: string;
  backpressured: boolean;
}

export interface UseTerminalSessionOptions {
  transport: StrataTransport;
  containerId: string;
  shell: TerminalShellId;
  termRef: RefObject<Terminal | null>;
  onSessionBound?: (sessionId: string) => void;
}

const RECONNECT_DELAYS_MS = [1000, 2000, 4000, 8000, 8000] as const;
const MAX_RECONNECT_ATTEMPTS = RECONNECT_DELAYS_MS.length;
const BACKPRESSURE_THROTTLE_MS = 500;

/**
 * Manages a WebSocket PTY session decoupled from xterm.js lifecycle,
 * featuring immediate post-open resize sync, exponential backoff auto-reconnect,
 * anti-duplication `term.reset()` on resume, and zero per-chunk React re-renders.
 */
export function useTerminalSession({
  transport,
  containerId,
  shell,
  termRef,
  onSessionBound,
}: UseTerminalSessionOptions) {
  const transportRef = useRef(transport);
  transportRef.current = transport;
  const containerIdRef = useRef(containerId);
  containerIdRef.current = containerId;
  const shellRef = useRef(shell);
  shellRef.current = shell;
  const onSessionBoundRef = useRef(onSessionBound);
  onSessionBoundRef.current = onSessionBound;

  const ptyRef = useRef<PtySessionHandle | null>(null);
  const sessionIdRef = useRef<string | undefined>(undefined);
  const flowRef = useRef(new PtyWatermarkController());
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttemptRef = useRef(0);
  const exitedCleanlyRef = useRef(false);
  const disposedRef = useRef(false);
  const lastBackpressureSyncRef = useRef(0);
  const lastBackpressureValueRef = useRef(false);

  const [state, setState] = useState<TerminalSessionState>({
    status: 'connecting',
    disconnectReason: null,
    reconnectAttempt: 0,
    maxReconnectAttempts: MAX_RECONNECT_ATTEMPTS,
    activeShell: shell,
    backpressured: false,
  });

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const connect = useCallback(
    (resumeId?: string, isAutoRetry = false) => {
      const term = termRef.current;
      if (!term || disposedRef.current) return;

      clearReconnectTimer();
      const prevHandle = ptyRef.current;
      ptyRef.current = null;
      prevHandle?.close();

      exitedCleanlyRef.current = false;
      flowRef.current = new PtyWatermarkController();
      lastBackpressureValueRef.current = false;

      if (!isAutoRetry) {
        reconnectAttemptRef.current = 0;
      }

      setState((prev) => ({
        ...prev,
        status: isAutoRetry ? 'reconnecting' : 'connecting',
        disconnectReason: isAutoRetry ? prev.disconnectReason : null,
        reconnectAttempt: reconnectAttemptRef.current,
        backpressured: false,
      }));

      const isResume = Boolean(resumeId);
      let handle: PtySessionHandle | null = null;
      handle = transportRef.current.openPty(
        containerIdRef.current,
        term.cols,
        term.rows,
        {
          onOpen: () => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            reconnectAttemptRef.current = 0;
            // Clear stale screen before replaying server ring buffer to prevent duplicate lines
            if (isResume) {
              term.reset();
            }
            // Immediately synchronize real xterm dimensions so PTY never remains stuck at 80x24
            handle?.sendResize(term.cols, term.rows);
            setState((prev) => ({
              ...prev,
              status: 'connected',
              disconnectReason: null,
              reconnectAttempt: 0,
            }));
          },
          onMeta: (meta) => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            sessionIdRef.current = meta.sessionId;
            onSessionBoundRef.current?.(meta.sessionId);
            if (meta.shell) {
              const detectedShell = meta.shell;
              setState((prev) =>
                prev.activeShell === detectedShell ? prev : { ...prev, activeShell: detectedShell },
              );
            }
          },
          onData: (chunk) => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            const flow = flowRef.current;
            flow.onReceive(chunk.byteLength);
            term.write(chunk, () => {
              const ackBytes = flow.onCommit(chunk.byteLength);
              if (ackBytes > 0 && ptyRef.current === handle) {
                ptyRef.current?.sendAck(ackBytes);
              }
              // Throttled backpressure check avoids per-chunk React re-render storms
              const now = Date.now();
              const bp = flow.isBackpressured();
              if (
                bp !== lastBackpressureValueRef.current &&
                now - lastBackpressureSyncRef.current >= BACKPRESSURE_THROTTLE_MS
              ) {
                lastBackpressureSyncRef.current = now;
                lastBackpressureValueRef.current = bp;
                setState((prev) =>
                  prev.backpressured === bp ? prev : { ...prev, backpressured: bp },
                );
              }
            });
          },
          onExit: (code) => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            exitedCleanlyRef.current = true;
            sessionIdRef.current = undefined;
            clearReconnectTimer();
            term.writeln(`\r\n\x1b[33m[进程已退出，退出码 ${code}]\x1b[0m`);
            setState((prev) => ({
              ...prev,
              status: 'disconnected',
              disconnectReason: `Shell 已退出 (退出码 ${code})`,
              reconnectAttempt: 0,
              backpressured: false,
            }));
          },
          onError: (msg) => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            setState((prev) => ({ ...prev, disconnectReason: msg }));
          },
          onClose: (reason) => {
            if (ptyRef.current !== handle || disposedRef.current) return;
            ptyRef.current = null;
            if (exitedCleanlyRef.current) return;

            const nextAttempt = reconnectAttemptRef.current + 1;
            const canAutoRetry = nextAttempt <= MAX_RECONNECT_ATTEMPTS;
            if (canAutoRetry) {
              reconnectAttemptRef.current = nextAttempt;
              const delayMs = RECONNECT_DELAYS_MS[nextAttempt - 1] ?? 8000;
              setState((prev) => ({
                ...prev,
                status: 'reconnecting',
                disconnectReason: `${reason} · ${(delayMs / 1000).toFixed(0)} 秒后自动重连 (${nextAttempt}/${MAX_RECONNECT_ATTEMPTS})`,
                reconnectAttempt: nextAttempt,
                backpressured: false,
              }));
              reconnectTimerRef.current = setTimeout(() => {
                if (!disposedRef.current) {
                  connect(sessionIdRef.current, true);
                }
              }, delayMs);
            } else {
              setState((prev) => ({
                ...prev,
                status: 'disconnected',
                disconnectReason: `${reason}（自动重连已达上限，请手动重连）`,
                reconnectAttempt: MAX_RECONNECT_ATTEMPTS,
                backpressured: false,
              }));
            }
          },
        },
        resumeId,
        shellRef.current,
      );
      ptyRef.current = handle;
    },
    [clearReconnectTimer, termRef],
  );

  const reconnectNow = useCallback(() => {
    connect(sessionIdRef.current, false);
  }, [connect]);

  const sendInput = useCallback((data: string | Uint8Array) => {
    ptyRef.current?.sendInput(data);
  }, []);

  const sendResize = useCallback((cols: number, rows: number) => {
    ptyRef.current?.sendResize(cols, rows);
  }, []);

  const dispose = useCallback(() => {
    disposedRef.current = true;
    clearReconnectTimer();
    const closing = ptyRef.current;
    ptyRef.current = null;
    closing?.close();
  }, [clearReconnectTimer]);

  // Reset disposed flag on mount (React StrictMode double-invoke safety)
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      clearReconnectTimer();
    };
  }, [clearReconnectTimer]);

  return {
    sessionState: state,
    connect,
    reconnectNow,
    sendInput,
    sendResize,
    dispose,
  };
}
