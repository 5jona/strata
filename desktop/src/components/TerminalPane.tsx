import { Badge, Button, cn, Input, StatusBanner } from '@gojona/ui';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { ArrowDown, ArrowUp, RefreshCw, Search, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { formatBytes } from '../lib/format.ts';
import type { PtySessionHandle, StrataTransport } from '../transport/httpWsTransport.ts';
import { type PtyFlowStats, PtyWatermarkController } from '../transport/ptyProtocol.ts';

export interface TerminalPaneProps {
  transport: StrataTransport;
  containerId: string;
  visible: boolean;
  onSessionBound?: (sessionId: string) => void;
}

/**
 * Single xterm.js v6 terminal pane with WebGL2 -> DOM fallback, CJK Unicode 11,
 * Ctrl+Shift+F search, Ctrl+Shift+C/V clipboard, watermark ACK flow control, and session reconnect.
 */
export function TerminalPane({
  transport,
  containerId,
  visible,
  onSessionBound,
}: TerminalPaneProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const ptyRef = useRef<PtySessionHandle | null>(null);
  const sessionIdRef = useRef<string | undefined>(undefined);
  const flowRef = useRef(new PtyWatermarkController());
  const onSessionBoundRef = useRef(onSessionBound);
  onSessionBoundRef.current = onSessionBound;

  const [status, setStatus] = useState<'connecting' | 'connected' | 'disconnected'>('connecting');
  const [disconnectReason, setDisconnectReason] = useState<string | null>(null);
  const [rendererKind, setRendererKind] = useState<'webgl2' | 'dom'>('dom');
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [flowStats, setFlowStats] = useState<PtyFlowStats>(() => flowRef.current.stats());

  const connectSession = useCallback(
    (resumeId?: string) => {
      const term = termRef.current;
      if (!term) return;
      const prevHandle = ptyRef.current;
      ptyRef.current = null;
      prevHandle?.close();
      flowRef.current = new PtyWatermarkController();
      setStatus('connecting');
      setDisconnectReason(null);

      let handle: PtySessionHandle | null = null;
      handle = transport.openPty(
        containerId,
        term.cols,
        term.rows,
        {
          onOpen: () => {
            if (ptyRef.current !== handle) return;
            setStatus('connected');
          },
          onMeta: (meta) => {
            if (ptyRef.current !== handle) return;
            sessionIdRef.current = meta.sessionId;
            onSessionBoundRef.current?.(meta.sessionId);
          },
          onData: (chunk) => {
            if (ptyRef.current !== handle) return;
            const flow = flowRef.current;
            flow.onReceive(chunk.byteLength);
            term.write(chunk, () => {
              const ackBytes = flow.onCommit(chunk.byteLength);
              if (ackBytes > 0 && ptyRef.current === handle) {
                ptyRef.current?.sendAck(ackBytes);
              }
              setFlowStats(flow.stats());
            });
          },
          onExit: (code) => {
            if (ptyRef.current !== handle) return;
            term.writeln(`\r\n\x1b[33m[进程已退出，退出码 ${code}]\x1b[0m`);
            sessionIdRef.current = undefined;
            setStatus('disconnected');
            setDisconnectReason(`Shell 已退出 (code=${code})`);
          },
          onClose: (reason) => {
            if (ptyRef.current !== handle) return;
            setStatus('disconnected');
            setDisconnectReason(reason);
          },
          onError: (msg) => {
            if (ptyRef.current !== handle) return;
            setStatus('disconnected');
            setDisconnectReason(msg);
          },
        },
        resumeId,
      );
      ptyRef.current = handle;
    },
    [containerId, transport],
  );

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      allowProposedApi: true,
      cursorBlink: true,
      fontSize: 13,
      lineHeight: 1.25,
      fontFamily: '"JetBrains Mono", "Cascadia Code", "Fira Code", monospace',
      scrollback: 5000,
      theme: {
        background: '#0d1117',
        foreground: '#e6edf3',
        cursor: '#38bdf8',
        selectionBackground: 'rgba(56, 189, 248, 0.28)',
      },
    });

    const fit = new FitAddon();
    const search = new SearchAddon();
    const unicode = new Unicode11Addon();
    const webLinks = new WebLinksAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(unicode);
    term.loadAddon(webLinks);
    term.unicode.activeVersion = '11';
    term.open(host);

    // Enable WebGL2 with automatic graceful fallback to DOM renderer on context loss.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        setRendererKind('dom');
      });
      term.loadAddon(webgl);
      setRendererKind('webgl2');
    } catch {
      setRendererKind('dom');
    }

    term.attachCustomKeyEventHandler((ev) => {
      if (ev.type !== 'keydown') return true;
      if (ev.ctrlKey && ev.shiftKey && ev.code === 'KeyF') {
        ev.preventDefault();
        setSearchOpen((prev) => !prev);
        return false;
      }
      if (ev.ctrlKey && ev.shiftKey && ev.code === 'KeyC') {
        const sel = term.getSelection();
        if (sel) void navigator.clipboard?.writeText(sel);
        return false;
      }
      if (ev.ctrlKey && ev.shiftKey && ev.code === 'KeyV') {
        void navigator.clipboard?.readText().then((txt) => {
          if (txt) ptyRef.current?.sendInput(txt);
        });
        return false;
      }
      return true;
    });

    const dataSub = term.onData((data) => ptyRef.current?.sendInput(data));
    const resizeSub = term.onResize(({ cols, rows }) => ptyRef.current?.sendResize(cols, rows));

    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;

    try {
      fit.fit();
    } catch {
      // Ignore initial zero-size container
    }

    const ro = new ResizeObserver(() => {
      if (!host.offsetWidth || !host.offsetHeight) return;
      try {
        fit.fit();
      } catch {
        // Ignore fit race during collapse
      }
    });
    ro.observe(host);

    connectSession();

    return () => {
      ro.disconnect();
      dataSub.dispose();
      resizeSub.dispose();
      const closing = ptyRef.current;
      ptyRef.current = null;
      closing?.close();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
  }, [connectSession]);

  useEffect(() => {
    if (visible && fitRef.current && termRef.current) {
      requestAnimationFrame(() => {
        try {
          fitRef.current?.fit();
          termRef.current?.focus();
        } catch {
          // Ignore layout race
        }
      });
    }
  }, [visible]);

  return (
    <div className={cn('relative flex h-full flex-col bg-[#0d1117]', !visible && 'hidden')}>
      {/* Disconnect / Reconnect Banner */}
      {status === 'disconnected' && (
        <div className="p-2">
          <StatusBanner
            tone="warning"
            title="终端连接已断开"
            description={disconnectReason ?? '点击重连将自动复用后端 128 KiB 环形缓冲会话'}
            action={
              <Button
                size="sm"
                variant="primary"
                onClick={() => connectSession(sessionIdRef.current)}
              >
                <RefreshCw className="size-3.5" />
                重新连接
              </Button>
            }
          />
        </div>
      )}

      {/* Search Bar Overlay (Ctrl+Shift+F) */}
      {searchOpen && (
        <div className="absolute top-3 right-4 z-20 flex items-center gap-1.5 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-card)] p-1.5 shadow-lg">
          <Search className="ml-1 size-3.5 text-[var(--text-secondary)]" />
          <Input
            mono
            autoFocus
            className="h-7 w-48 text-xs"
            value={searchQuery}
            placeholder="搜索终端输出…"
            onChange={(e) => {
              setSearchQuery(e.target.value);
              if (e.target.value) searchRef.current?.findNext(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                if (e.shiftKey) searchRef.current?.findPrevious(searchQuery);
                else searchRef.current?.findNext(searchQuery);
              } else if (e.key === 'Escape') {
                setSearchOpen(false);
              }
            }}
          />
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="上一处匹配"
            onClick={() => searchRef.current?.findPrevious(searchQuery)}
          >
            <ArrowUp className="size-3.5" />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="下一处匹配"
            onClick={() => searchRef.current?.findNext(searchQuery)}
          >
            <ArrowDown className="size-3.5" />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="关闭搜索"
            onClick={() => setSearchOpen(false)}
          >
            <X className="size-3.5" />
          </Button>
        </div>
      )}

      {/* Terminal Viewport */}
      <div ref={hostRef} className="strata-xterm-host min-h-0 flex-1" />

      {/* Flow-Control & Renderer Status Strip */}
      <div className="flex items-center justify-between gap-2 border-t border-white/10 bg-[#0b0f14] px-3 py-1 font-mono text-[11px] text-slate-400">
        <div className="flex items-center gap-2">
          <Badge variant={status === 'connected' ? 'success' : 'warning'}>
            {status === 'connected' ? 'PTY 已连接' : status === 'connecting' ? '连接中' : '已断开'}
          </Badge>
          <span>渲染: {rendererKind === 'webgl2' ? 'WebGL2' : 'DOM'}</span>
          <span>接收: {formatBytes(flowStats.totalReceivedBytes)}</span>
          <span>ACK: {formatBytes(flowStats.totalAckedBytes)}</span>
          {flowStats.backpressured && <Badge variant="warning">流控背压中</Badge>}
        </div>
        <div className="flex items-center gap-2">
          <span>Ctrl+Shift+F 搜索 · Ctrl+Shift+C/V 复制粘贴</span>
          <button
            type="button"
            className="text-sky-400 hover:underline"
            onClick={() => setSearchOpen((v) => !v)}
          >
            搜索
          </button>
        </div>
      </div>
    </div>
  );
}
