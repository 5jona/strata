import { cn, colorWithAlpha, readThemeToken, useTheme } from '@gojona/ui';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { type ITheme, Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { PtySessionHandle, StrataTransport } from '../transport/httpWsTransport.ts';
import { type PtyFlowStats, PtyWatermarkController } from '../transport/ptyProtocol.ts';
import {
  TerminalDisconnectBanner,
  TerminalSearchBar,
  TerminalStatusStrip,
} from './TerminalOverlays.tsx';

export interface TerminalPaneProps {
  transport: StrataTransport;
  containerId: string;
  visible: boolean;
  onSessionBound?: (sessionId: string) => void;
}

function buildTerminalTheme(resolvedScheme: 'light' | 'dark'): ITheme {
  const background = readThemeToken('surface-canvas', resolvedScheme, 'Canvas');
  const foreground = readThemeToken('text-primary', resolvedScheme, 'CanvasText');
  const cursor = readThemeToken('accent', resolvedScheme, 'Highlight');
  return {
    background,
    foreground,
    cursor,
    selectionBackground: colorWithAlpha(cursor, 0.28),
  };
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
  const { resolvedScheme, themeId } = useTheme();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const ptyRef = useRef<PtySessionHandle | null>(null);
  const sessionIdRef = useRef<string | undefined>(undefined);
  const flowRef = useRef(new PtyWatermarkController());
  const onSessionBoundRef = useRef(onSessionBound);
  onSessionBoundRef.current = onSessionBound;
  const schemeRef = useRef(resolvedScheme);
  schemeRef.current = resolvedScheme;

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
            if (ptyRef.current === handle) setStatus('connected');
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
              if (ackBytes > 0 && ptyRef.current === handle) ptyRef.current?.sendAck(ackBytes);
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
      theme: buildTerminalTheme(schemeRef.current),
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
    if (termRef.current && themeId) {
      termRef.current.options.theme = buildTerminalTheme(resolvedScheme);
    }
  }, [resolvedScheme, themeId]);

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
    <div
      className={cn(
        'relative flex h-full flex-col bg-[var(--surface-canvas)]',
        !visible && 'hidden',
      )}
    >
      {status === 'disconnected' && (
        <TerminalDisconnectBanner
          reason={disconnectReason}
          onReconnect={() => connectSession(sessionIdRef.current)}
        />
      )}

      {searchOpen && (
        <TerminalSearchBar
          query={searchQuery}
          onQueryChange={(next) => {
            setSearchQuery(next);
            if (next) searchRef.current?.findNext(next);
          }}
          onFindNext={() => searchRef.current?.findNext(searchQuery)}
          onFindPrevious={() => searchRef.current?.findPrevious(searchQuery)}
          onClose={() => setSearchOpen(false)}
        />
      )}

      <div ref={hostRef} className="strata-xterm-host min-h-0 flex-1" />

      <TerminalStatusStrip
        status={status}
        rendererKind={rendererKind}
        flowStats={flowStats}
        onToggleSearch={() => setSearchOpen((v) => !v)}
      />
    </div>
  );
}
