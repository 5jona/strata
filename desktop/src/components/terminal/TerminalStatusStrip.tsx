import { cn } from '@gojona/ui';
import {
  Columns2,
  ExternalLink,
  Minus,
  Plus,
  Rows2,
  Search,
  SlidersHorizontal,
} from 'lucide-react';
import { formatShellLabel, MAX_FONT_SIZE, MIN_FONT_SIZE } from './terminalPrefs.ts';
import type { TerminalConnectionStatus } from './useTerminalSession.ts';

export interface TerminalStatusStripProps {
  status: TerminalConnectionStatus;
  containerId: string;
  shell: string;
  cols: number;
  rows: number;
  fontSize: number;
  rendererKind: 'webgl2' | 'dom';
  backpressured?: boolean;
  canSplit: boolean;
  canExternalSsh: boolean;
  onStepFontSize: (delta: number) => void;
  onResetFontSize: () => void;
  onSplitRight: () => void;
  onSplitDown: () => void;
  onToggleSearch: () => void;
  onOpenSettings: () => void;
  onOpenExternalSsh: () => void;
}

const STATUS_META: Record<TerminalConnectionStatus, { label: string; dotClass: string }> = {
  connected: { label: '已连接', dotClass: 'bg-emerald-500' },
  connecting: { label: '连接中', dotClass: 'bg-amber-400 animate-pulse' },
  reconnecting: { label: '自动重连中', dotClass: 'bg-amber-500 animate-pulse' },
  disconnected: { label: '已断开', dotClass: 'bg-rose-500' },
};

/** Clean modern bottom status bar for the active focused terminal pane. */
export function TerminalStatusStrip({
  status,
  containerId,
  shell,
  cols,
  rows,
  fontSize,
  rendererKind,
  backpressured,
  canSplit,
  canExternalSsh,
  onStepFontSize,
  onResetFontSize,
  onSplitRight,
  onSplitDown,
  onToggleSearch,
  onOpenSettings,
  onOpenExternalSsh,
}: TerminalStatusStripProps) {
  const statusInfo = STATUS_META[status];

  return (
    <div className="flex h-7 shrink-0 items-center justify-between gap-2 border-t border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 text-[11px] text-[var(--text-secondary)]">
      {/* Left: connection dot, container + shell, dimensions, renderer */}
      <div className="flex min-w-0 items-center gap-2.5">
        <span className="flex shrink-0 items-center gap-1.5 font-medium text-[var(--text-primary)]">
          <span className={cn('size-2 rounded-full', statusInfo.dotClass)} />
          {statusInfo.label}
        </span>

        <span className="h-3 w-px bg-[var(--border-hairline)]" />

        <span className="truncate font-mono text-[var(--text-primary)]">
          {containerId}
          <span className="mx-1 text-[var(--text-tertiary)]">·</span>
          <span className="text-[var(--accent)]">{formatShellLabel(shell)}</span>
        </span>

        {cols > 0 && rows > 0 && (
          <span className="hidden font-mono text-[var(--text-tertiary)] sm:inline">
            {cols} × {rows}
          </span>
        )}

        <span className="hidden rounded bg-[var(--surface-card)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)] md:inline">
          {rendererKind === 'webgl2' ? 'WebGL2' : 'DOM'}
        </span>

        {backpressured && (
          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-500">
            高负载缓冲中
          </span>
        )}
      </div>

      {/* Right: font size controls, splits, search, appearance settings, external SSH */}
      <div className="flex shrink-0 items-center gap-1">
        <div className="flex items-center rounded border border-[var(--border-hairline)] bg-[var(--surface-card)]">
          <button
            type="button"
            title="缩小字号 (Ctrl+-)"
            aria-label="缩小字号"
            disabled={fontSize <= MIN_FONT_SIZE}
            onClick={() => onStepFontSize(-1)}
            className="px-1 py-0.5 hover:text-[var(--text-primary)] disabled:opacity-40"
          >
            <Minus className="size-3" />
          </button>
          <button
            type="button"
            title="重置默认字号 (Ctrl+0)"
            onClick={onResetFontSize}
            className="px-1 font-mono text-[10px] hover:text-[var(--accent)]"
          >
            {fontSize}px
          </button>
          <button
            type="button"
            title="放大字号 (Ctrl+=)"
            aria-label="放大字号"
            disabled={fontSize >= MAX_FONT_SIZE}
            onClick={() => onStepFontSize(1)}
            className="px-1 py-0.5 hover:text-[var(--text-primary)] disabled:opacity-40"
          >
            <Plus className="size-3" />
          </button>
        </div>

        <span className="mx-0.5 h-3 w-px bg-[var(--border-hairline)]" />

        <button
          type="button"
          title="向右分屏 (Ctrl+Shift+D)"
          aria-label="向右分屏"
          disabled={!canSplit}
          onClick={onSplitRight}
          className="rounded p-1 hover:bg-[var(--surface-card)] hover:text-[var(--text-primary)] disabled:opacity-35"
        >
          <Columns2 className="size-3.5" />
        </button>

        <button
          type="button"
          title="向下分屏 (Ctrl+Shift+E)"
          aria-label="向下分屏"
          disabled={!canSplit}
          onClick={onSplitDown}
          className="rounded p-1 hover:bg-[var(--surface-card)] hover:text-[var(--text-primary)] disabled:opacity-35"
        >
          <Rows2 className="size-3.5" />
        </button>

        <button
          type="button"
          title="查找内容 (Ctrl+Shift+F)"
          aria-label="查找内容"
          onClick={onToggleSearch}
          className="rounded p-1 hover:bg-[var(--surface-card)] hover:text-[var(--text-primary)]"
        >
          <Search className="size-3.5" />
        </button>

        <button
          type="button"
          title="终端外观与交互设置"
          aria-label="终端设置"
          onClick={onOpenSettings}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-[var(--surface-card)] hover:text-[var(--accent)]"
        >
          <SlidersHorizontal className="size-3.5" />
          <span className="hidden sm:inline">外观</span>
        </button>

        <button
          type="button"
          title="在系统原生终端打开 SSH"
          aria-label="外部终端 SSH"
          disabled={!canExternalSsh}
          onClick={onOpenExternalSsh}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-[var(--surface-card)] hover:text-[var(--accent)] disabled:opacity-35"
        >
          <ExternalLink className="size-3.5" />
          <span className="hidden md:inline">外部 SSH</span>
        </button>
      </div>
    </div>
  );
}
