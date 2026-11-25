import type { CopyFeedbackStatus } from '@gojona/ui';
import { Badge, Button, Card, ControlTile } from '@gojona/ui';
import { MousePointerClick, Play, RotateCw, Square, Terminal, Tv, Wifi } from 'lucide-react';
import { formatBytes, formatUptime, PHASE_META } from '../shared/strataFormat.ts';
import type { ContainerStatusSnapshot, HudStateResult } from '../types.ts';
import type { QuickLifecycleAction } from './useStrataQuickPanel.ts';

export interface QuickControlSectionProps {
  container: ContainerStatusSnapshot;
  hudState: HudStateResult;
  busyAction: string | null;
  copyStatus: CopyFeedbackStatus;
  onLifecycle: (action: QuickLifecycleAction) => Promise<void>;
  onCopySsh: () => Promise<void>;
  onSyncDns: () => Promise<void>;
  onUpdateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
}

/** Container quick lifecycle card and 2x2 shortcut ControlTile grid for SideVeil. */
export function QuickControlSection({
  container,
  hudState,
  busyAction,
  copyStatus,
  onLifecycle,
  onCopySsh,
  onSyncDns,
  onUpdateHud,
}: QuickControlSectionProps) {
  const phaseInfo = PHASE_META[container.phase];
  const isRunning = container.phase === 'running';
  const busy = busyAction !== null;
  const imageMissing = !container.disk.imageExists;

  return (
    <>
      <Card className="flex flex-col gap-2.5 p-3">
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-sm font-semibold text-[var(--text-primary)]">
              {container.hostname}
            </span>
            <Badge className="shrink-0" variant={phaseInfo.variant} dot pulse={isRunning}>
              {phaseInfo.label}
            </Badge>
          </div>
          <p className="strata-meta-line mt-0.5 truncate font-mono text-[11px] text-[var(--text-secondary)]">
            {isRunning
              ? `PID ${container.nsPid ?? '-'} · ${formatUptime(container.uptimeMs)} · ${formatBytes(container.disk.imageAllocatedBytes)}`
              : `镜像 ${formatBytes(container.disk.imageAllocatedBytes)} · ${container.imagePath}`}
          </p>
        </div>

        {imageMissing ? (
          <p className="text-[11px] text-[var(--warning)]">
            未检测到 rootfs 镜像，请通过 Strata Desktop 或 strata CLI 初始化。
          </p>
        ) : null}

        <div className="grid grid-cols-3 gap-1.5">
          <Button
            size="sm"
            variant="primary"
            disabled={busy || isRunning || imageMissing}
            aria-label="启动容器"
            onClick={() => void onLifecycle('start')}
          >
            <Play className="size-3.5" />
            启动
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || !isRunning}
            aria-label="停止容器"
            onClick={() => void onLifecycle('stop')}
          >
            <Square className="size-3.5" />
            停止
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || imageMissing}
            aria-label="重启容器"
            onClick={() => void onLifecycle('restart')}
          >
            <RotateCw className="size-3.5" />
            重启
          </Button>
        </div>
      </Card>

      <div className="grid grid-cols-2 gap-2">
        <ControlTile
          label={copyStatus === 'copied' ? '已复制 SSH' : '复制 SSH'}
          subtitle={`:${container.sshPort}`}
          icon={<Terminal className="size-4" />}
          active={copyStatus === 'copied'}
          tone="success"
          onClick={() => void onCopySsh()}
        />
        <ControlTile
          label="同步 DNS"
          subtitle={container.mounted ? '更新解析与桥接' : '容器未挂载'}
          icon={<Wifi className="size-4" />}
          disabled={busy || !container.mounted}
          loading={busyAction === 'dns'}
          onClick={() => void onSyncDns()}
        />
        <ControlTile
          label="原生 HUD"
          subtitle={hudState.enabled ? '状态条开启' : '状态条关闭'}
          icon={<Tv className="size-4" />}
          active={hudState.enabled}
          tone="success"
          onClick={() => void onUpdateHud({ enabled: !hudState.enabled })}
        />
        <ControlTile
          label="HUD 穿透"
          subtitle={
            hudState.enabled ? (hudState.clickThrough ? '忽略触摸' : '允许触摸') : '需先开启 HUD'
          }
          icon={<MousePointerClick className="size-4" />}
          active={hudState.enabled && hudState.clickThrough}
          disabled={!hudState.enabled}
          tone="accent"
          onClick={() => void onUpdateHud({ clickThrough: !hudState.clickThrough })}
        />
      </div>
    </>
  );
}
