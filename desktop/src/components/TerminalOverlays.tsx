import { Button, cn, Input, StatusBanner } from '@gojona/ui';
import {
  ArrowDown,
  ArrowUp,
  CaseSensitive,
  RefreshCw,
  Regex,
  Search,
  WholeWord,
  X,
} from 'lucide-react';
import type { TerminalConnectionStatus } from './terminal/useTerminalSession.ts';

export {
  TerminalStatusStrip,
  type TerminalStatusStripProps,
} from './terminal/TerminalStatusStrip.tsx';

export interface TerminalDisconnectBannerProps {
  status: TerminalConnectionStatus;
  reason: string | null;
  onReconnect: () => void;
}

/** Reconnect banner shown when the PTY WebSocket disconnects or is auto-retrying. */
export function TerminalDisconnectBanner({
  status,
  reason,
  onReconnect,
}: TerminalDisconnectBannerProps) {
  const isRetrying = status === 'reconnecting';
  return (
    <div className="p-2">
      <StatusBanner
        tone={isRetrying ? 'info' : 'warning'}
        title={isRetrying ? '终端正在自动重连…' : '终端连接已断开'}
        description={reason ?? '点击重新连接即可恢复会话'}
        action={
          <Button size="sm" variant="primary" onClick={onReconnect}>
            <RefreshCw className={cn('size-3.5', isRetrying && 'animate-spin')} />
            立即重连
          </Button>
        }
      />
    </div>
  );
}

export interface TerminalSearchOptions {
  caseSensitive: boolean;
  regex: boolean;
  wholeWord: boolean;
}

export interface TerminalSearchMatchStats {
  index: number;
  count: number;
}

export interface TerminalSearchBarProps {
  query: string;
  options: TerminalSearchOptions;
  matchStats: TerminalSearchMatchStats;
  onQueryChange: (next: string) => void;
  onOptionsChange: (patch: Partial<TerminalSearchOptions>) => void;
  onFindNext: () => void;
  onFindPrevious: () => void;
  onClose: () => void;
}

/** Floating Ctrl+Shift+F search overlay with live match counter and option toggles. */
export function TerminalSearchBar({
  query,
  options,
  matchStats,
  onQueryChange,
  onOptionsChange,
  onFindNext,
  onFindPrevious,
  onClose,
}: TerminalSearchBarProps) {
  const countLabel = !query
    ? ''
    : matchStats.count === 0
      ? '无匹配'
      : matchStats.index < 0
        ? `${matchStats.count}+`
        : `${matchStats.index + 1} / ${matchStats.count}`;

  return (
    <div className="strata-term-menu absolute top-2.5 right-3 z-20 flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-card)]/95 p-1.5 shadow-lg backdrop-blur-md">
      <Search className="ml-1 size-3.5 shrink-0 text-[var(--text-secondary)]" />
      <Input
        mono
        autoFocus
        className="h-7 w-44 text-xs"
        value={query}
        placeholder="搜索终端内容…"
        onChange={(e) => onQueryChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            if (e.shiftKey) onFindPrevious();
            else onFindNext();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            onClose();
          }
        }}
      />

      <button
        type="button"
        title="区分大小写"
        aria-label="区分大小写"
        aria-pressed={options.caseSensitive}
        onClick={() => onOptionsChange({ caseSensitive: !options.caseSensitive })}
        className={cn(
          'rounded p-1 transition-colors',
          options.caseSensitive
            ? 'bg-[var(--accent)]/20 text-[var(--accent)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--surface-elevated)]',
        )}
      >
        <CaseSensitive className="size-3.5" />
      </button>

      <button
        type="button"
        title="全词匹配"
        aria-label="全词匹配"
        aria-pressed={options.wholeWord}
        onClick={() => onOptionsChange({ wholeWord: !options.wholeWord })}
        className={cn(
          'rounded p-1 transition-colors',
          options.wholeWord
            ? 'bg-[var(--accent)]/20 text-[var(--accent)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--surface-elevated)]',
        )}
      >
        <WholeWord className="size-3.5" />
      </button>

      <button
        type="button"
        title="正则表达式"
        aria-label="正则表达式"
        aria-pressed={options.regex}
        onClick={() => onOptionsChange({ regex: !options.regex })}
        className={cn(
          'rounded p-1 transition-colors',
          options.regex
            ? 'bg-[var(--accent)]/20 text-[var(--accent)]'
            : 'text-[var(--text-secondary)] hover:bg-[var(--surface-elevated)]',
        )}
      >
        <Regex className="size-3.5" />
      </button>

      {countLabel && (
        <span className="min-w-12 px-1 text-center font-mono text-[11px] text-[var(--text-secondary)]">
          {countLabel}
        </span>
      )}

      <Button size="icon-sm" variant="ghost" aria-label="上一处匹配" onClick={onFindPrevious}>
        <ArrowUp className="size-3.5" />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label="下一处匹配" onClick={onFindNext}>
        <ArrowDown className="size-3.5" />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label="关闭搜索" onClick={onClose}>
        <X className="size-3.5" />
      </Button>
    </div>
  );
}

/** Floating hint shown when hovering a clickable terminal hyperlink. */
export function TerminalLinkTooltip({
  url,
  x,
  y,
  isMac,
}: {
  url: string;
  x: number;
  y: number;
  isMac: boolean;
}) {
  const maxLeft = (typeof window !== 'undefined' ? window.innerWidth : 1280) - 280;
  return (
    <div
      style={{ left: Math.min(x + 12, maxLeft), top: Math.max(8, y - 34) }}
      className="pointer-events-none fixed z-40 max-w-xs truncate rounded-md border border-[var(--border-hairline)] bg-[var(--surface-card)]/95 px-2 py-1 text-[11px] text-[var(--text-primary)] shadow-md"
    >
      <span className="mr-1.5 font-medium text-[var(--accent)]">
        {isMac ? '⌘+点击打开' : 'Ctrl+点击打开'}
      </span>
      <span className="font-mono text-[var(--text-secondary)]">{url}</span>
    </div>
  );
}
