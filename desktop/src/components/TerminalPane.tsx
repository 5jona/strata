import { cn, useTheme } from '@gojona/ui';
import type { ISearchOptions } from '@xterm/addon-search';
import type { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import {
  TerminalDisconnectBanner,
  TerminalLinkTooltip,
  TerminalSearchBar,
  type TerminalSearchMatchStats,
  type TerminalSearchOptions,
} from './TerminalOverlays.tsx';
import {
  TerminalContextMenu,
  type TerminalContextMenuState,
} from './terminal/TerminalContextMenu.tsx';
import {
  isMacPlatform,
  readClipboardText,
  writeClipboardText,
} from './terminal/terminalPlatform.ts';
import {
  DEFAULT_TERMINAL_PREFS,
  type TerminalPrefs,
  type TerminalShellId,
} from './terminal/terminalPrefs.ts';
import { resolveTerminalTheme } from './terminal/terminalThemes.ts';
import {
  type TerminalConnectionStatus,
  useTerminalSession,
} from './terminal/useTerminalSession.ts';
import { useXtermInstance } from './terminal/useXtermInstance.ts';

export interface TerminalPaneMetrics {
  status: TerminalConnectionStatus;
  containerId: string;
  activeShell: string;
  cols: number;
  rows: number;
  rendererKind: 'webgl2' | 'dom';
  backpressured: boolean;
}

export interface TerminalPaneActions {
  toggleSearch: () => void;
  focus: () => void;
  fit: () => void;
}

export interface TerminalPaneProps {
  transport: StrataTransport;
  containerId: string;
  shell?: TerminalShellId;
  visible: boolean;
  focused?: boolean;
  canSplit?: boolean;
  prefs?: TerminalPrefs;
  onStepFontSize?: (delta: number) => void;
  onResetFontSize?: () => void;
  onSessionBound?: (sessionId: string) => void;
  onTitleChange?: (title: string) => void;
  onMetricsChange?: (metrics: TerminalPaneMetrics) => void;
  onRegisterActions?: (actions: TerminalPaneActions | null) => void;
  onSplitRight?: () => void;
  onSplitDown?: () => void;
  onNewTabShortcut?: () => void;
  onClosePaneShortcut?: () => void;
  onOpenExternalSsh?: () => void;
}

/**
 * Single xterm.js v6 terminal pane composing `useXtermInstance` and `useTerminalSession`
 * with search overlay, link hover hint, right-click menu, and reconnect banner.
 */
export function TerminalPane({
  transport,
  containerId,
  shell = 'auto',
  visible,
  focused = true,
  canSplit = false,
  prefs = DEFAULT_TERMINAL_PREFS,
  onStepFontSize,
  onResetFontSize,
  onSessionBound,
  onTitleChange,
  onMetricsChange,
  onRegisterActions,
  onSplitRight,
  onSplitDown,
  onNewTabShortcut,
  onClosePaneShortcut,
  onOpenExternalSsh,
}: TerminalPaneProps) {
  const { resolvedScheme } = useTheme();
  const resolvedTheme = useMemo(
    () => resolveTerminalTheme(prefs.themeId, resolvedScheme),
    [prefs.themeId, resolvedScheme],
  );

  const hostRef = useRef<HTMLDivElement | null>(null);
  const sharedTermRef = useRef<Terminal | null>(null);
  const refs = useRef({ resolvedTheme, prefs, onRegisterActions, onMetricsChange });
  refs.current = { resolvedTheme, prefs, onRegisterActions, onMetricsChange };

  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchOpts, setSearchOpts] = useState<TerminalSearchOptions>({
    caseSensitive: false,
    regex: false,
    wholeWord: false,
  });
  const [matchStats, setMatchStats] = useState<TerminalSearchMatchStats>({ index: -1, count: 0 });
  const [contextMenu, setContextMenu] = useState<TerminalContextMenuState | null>(null);

  const { sessionState, connect, reconnectNow, sendInput, sendResize, dispose } =
    useTerminalSession({
      transport,
      containerId,
      shell,
      termRef: sharedTermRef,
      onSessionBound,
    });

  const { termRef, searchRef, rendererKind, dims, hoverLink, safeFit } = useXtermInstance({
    hostRef,
    prefs,
    resolvedTheme,
    visible,
    focused,
    onData: sendInput,
    onResize: sendResize,
    onTitleChange,
    onSearchResults: (ev) => setMatchStats({ index: ev.resultIndex, count: ev.resultCount }),
    onReady: () => {
      sharedTermRef.current = termRef.current;
      connect();
    },
    onDispose: () => {
      dispose();
      sharedTermRef.current = null;
    },
    getKeyHandlers: () => ({
      onToggleSearch: () => setSearchOpen((v) => !v),
      onStepFontSize: (delta) => onStepFontSize?.(delta),
      onResetFontSize: () => onResetFontSize?.(),
      onClearTerminal: () => termRef.current?.clear(),
      onNewTabShortcut,
      onClosePaneShortcut,
      onSplitRightShortcut: onSplitRight,
      onSplitDownShortcut: onSplitDown,
    }),
  });

  // Keep sharedTermRef aligned synchronously after xterm mount
  sharedTermRef.current = termRef.current;

  const runSearch = useCallback(
    (query: string, opts: TerminalSearchOptions, direction: 'next' | 'prev' = 'next') => {
      const addon = searchRef.current;
      if (!addon) return;
      if (!query) {
        addon.clearDecorations();
        setMatchStats({ index: -1, count: 0 });
        return;
      }
      const searchOptions: ISearchOptions = {
        ...opts,
        decorations: refs.current.resolvedTheme.searchColors,
      };
      if (direction === 'prev') addon.findPrevious(query, searchOptions);
      else addon.findNext(query, searchOptions);
    },
    [searchRef],
  );

  useEffect(() => {
    refs.current.onRegisterActions?.({
      toggleSearch: () => setSearchOpen((v) => !v),
      focus: () => termRef.current?.focus(),
      fit: safeFit,
    });
    return () => refs.current.onRegisterActions?.(null);
  }, [safeFit, termRef]);

  useEffect(() => {
    refs.current.onMetricsChange?.({
      status: sessionState.status,
      containerId,
      activeShell: sessionState.activeShell,
      cols: dims.cols,
      rows: dims.rows,
      rendererKind,
      backpressured: sessionState.backpressured,
    });
  }, [containerId, dims, rendererKind, sessionState]);

  const handleContextMenu = useCallback(
    (ev: React.MouseEvent<HTMLDivElement>) => {
      ev.preventDefault();
      const term = termRef.current;
      if (!term) return;
      if (refs.current.prefs.rightClickPaste && !ev.shiftKey) {
        void readClipboardText().then((txt) => {
          if (txt) term.paste(txt);
        });
        return;
      }
      setContextMenu({ x: ev.clientX, y: ev.clientY, hasSelection: term.hasSelection() });
    },
    [termRef],
  );

  const handleCloseSearch = useCallback(() => {
    setSearchOpen(false);
    searchRef.current?.clearDecorations();
    setMatchStats({ index: -1, count: 0 });
    termRef.current?.focus();
  }, [searchRef, termRef]);

  const showBanner =
    sessionState.status === 'disconnected' || sessionState.status === 'reconnecting';

  return (
    <div
      role="application"
      aria-label={`终端 ${containerId}`}
      className={cn('relative flex h-full w-full flex-col overflow-hidden', !visible && 'hidden')}
      style={{ backgroundColor: resolvedTheme.background }}
      onContextMenu={handleContextMenu}
    >
      {showBanner && (
        <TerminalDisconnectBanner
          status={sessionState.status}
          reason={sessionState.disconnectReason}
          onReconnect={reconnectNow}
        />
      )}

      {searchOpen && (
        <TerminalSearchBar
          query={searchQuery}
          options={searchOpts}
          matchStats={matchStats}
          onQueryChange={(next) => {
            setSearchQuery(next);
            runSearch(next, searchOpts, 'next');
          }}
          onOptionsChange={(patch) => {
            const nextOpts = { ...searchOpts, ...patch };
            setSearchOpts(nextOpts);
            if (searchQuery) runSearch(searchQuery, nextOpts, 'next');
          }}
          onFindNext={() => runSearch(searchQuery, searchOpts, 'next')}
          onFindPrevious={() => runSearch(searchQuery, searchOpts, 'prev')}
          onClose={handleCloseSearch}
        />
      )}

      <div ref={hostRef} className="strata-xterm-host min-h-0 flex-1" />

      {hoverLink && (
        <TerminalLinkTooltip
          url={hoverLink.url}
          x={hoverLink.x}
          y={hoverLink.y}
          isMac={isMacPlatform()}
        />
      )}

      <TerminalContextMenu
        menu={contextMenu}
        canSplit={canSplit}
        onClose={() => setContextMenu(null)}
        onCopy={() => {
          const sel = termRef.current?.getSelection();
          if (sel) void writeClipboardText(sel);
        }}
        onPaste={() =>
          void readClipboardText().then((txt) => {
            if (txt) termRef.current?.paste(txt);
          })
        }
        onClear={() => termRef.current?.clear()}
        onSearch={() => setSearchOpen(true)}
        onSplitRight={() => onSplitRight?.()}
        onSplitDown={() => onSplitDown?.()}
        onReset={() => termRef.current?.reset()}
        onOpenExternalSsh={onOpenExternalSsh}
      />
    </div>
  );
}
