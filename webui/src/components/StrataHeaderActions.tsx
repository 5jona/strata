import type { ContainerPresentationMode } from '@pochita/sideveil-sdk';
import { Badge, Button, cn } from '@pochita/ui';
import { Lock, Maximize2, PanelRight, Pin, RefreshCw, Unlock, X } from 'lucide-react';

/** Props accepted by `StrataHeaderActions`. */
export interface StrataHeaderActionsProps {
  isNativeHost: boolean;
  backendOnline: boolean;
  backendState: string;
  mode: ContainerPresentationMode;
  isPinned: boolean;
  clickThrough: boolean;
  loading: boolean;
  onToggleClickThrough: () => void;
  onCycleMode: () => void;
  onRefresh: () => void;
  onDismiss: () => void;
}

/** Trailing AppShell action cluster displaying status badge, window mode, refresh, and close controls. */
export function StrataHeaderActions({
  isNativeHost,
  backendOnline,
  backendState,
  mode,
  isPinned,
  clickThrough,
  loading,
  onToggleClickThrough,
  onCycleMode,
  onRefresh,
  onDismiss,
}: StrataHeaderActionsProps) {
  const modeLabel = mode === 'wide' ? '切换到悬浮窗' : isPinned ? '返回侧边抽屉' : '切换到宽屏模式';
  const ModeIcon = mode === 'wide' ? Pin : isPinned ? PanelRight : Maximize2;

  return (
    <div className="strata-header-actions flex items-center gap-1">
      <Badge
        className="shrink-0"
        variant={backendOnline ? 'success' : 'warning'}
        dot
        pulse={backendOnline}
      >
        {!isNativeHost ? '预览' : backendState === 'online' ? '在线' : '离线'}
      </Badge>
      {isPinned ? (
        <Button
          size="icon-sm"
          variant={clickThrough ? 'pill' : 'ghost'}
          aria-label={clickThrough ? '退出点击穿透' : '开启浮窗点击穿透'}
          onClick={onToggleClickThrough}
        >
          {clickThrough ? <Lock className="size-3.5" /> : <Unlock className="size-3.5" />}
        </Button>
      ) : null}
      <Button size="icon-sm" variant="ghost" aria-label={modeLabel} onClick={onCycleMode}>
        <ModeIcon className="size-3.5" />
      </Button>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="刷新容器状态"
        disabled={loading}
        onClick={onRefresh}
      >
        <RefreshCw className={cn('size-3.5', loading && 'animate-spin')} />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label="关闭 Strata 面板" onClick={onDismiss}>
        <X className="size-4" />
      </Button>
    </div>
  );
}
