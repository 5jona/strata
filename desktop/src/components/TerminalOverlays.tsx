import { Badge, Button, Input, StatusBanner } from '@gojona/ui';
import { ArrowDown, ArrowUp, RefreshCw, Search, X } from 'lucide-react';
import { formatBytes } from '../lib/format.ts';
import type { PtyFlowStats } from '../transport/ptyProtocol.ts';

export interface TerminalDisconnectBannerProps {
  reason: string | null;
  onReconnect: () => void;
}

/** Reconnect banner shown when the PTY WebSocket disconnects. */
export function TerminalDisconnectBanner({ reason, onReconnect }: TerminalDisconnectBannerProps) {
  return (
    <div className="p-2">
      <StatusBanner
        tone="warning"
        title="终端连接已断开"
        description={reason ?? '点击重连将自动复用后端 128 KiB 环形缓冲会话'}
        action={
          <Button size="sm" variant="primary" onClick={onReconnect}>
            <RefreshCw className="size-3.5" />
            重新连接
          </Button>
        }
      />
    </div>
  );
}

export interface TerminalSearchBarProps {
  query: string;
  onQueryChange: (next: string) => void;
  onFindNext: () => void;
  onFindPrevious: () => void;
  onClose: () => void;
}

/** Floating Ctrl+Shift+F search overlay for xterm.js output. */
export function TerminalSearchBar({
  query,
  onQueryChange,
  onFindNext,
  onFindPrevious,
  onClose,
}: TerminalSearchBarProps) {
  return (
    <div className="absolute top-3 right-4 z-20 flex items-center gap-1.5 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-card)] p-1.5 shadow-lg">
      <Search className="ml-1 size-3.5 text-[var(--text-secondary)]" />
      <Input
        mono
        autoFocus
        className="h-7 w-48 text-xs"
        value={query}
        placeholder="搜索终端输出…"
        onChange={(e) => onQueryChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            if (e.shiftKey) onFindPrevious();
            else onFindNext();
          } else if (e.key === 'Escape') {
            onClose();
          }
        }}
      />
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

export interface TerminalStatusStripProps {
  status: 'connecting' | 'connected' | 'disconnected';
  rendererKind: 'webgl2' | 'dom';
  flowStats: PtyFlowStats;
  onToggleSearch: () => void;
}

/** Bottom flow-control watermark and renderer status bar styled with semantic tokens. */
export function TerminalStatusStrip({
  status,
  rendererKind,
  flowStats,
  onToggleSearch,
}: TerminalStatusStripProps) {
  return (
    <div className="flex items-center justify-between gap-2 border-t border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-1 font-mono text-[11px] text-[var(--text-secondary)]">
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
          className="text-[var(--accent)] hover:underline"
          onClick={onToggleSearch}
        >
          搜索
        </button>
      </div>
    </div>
  );
}
