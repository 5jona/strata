import { Badge, Button, Card, MetricTile, SettingRow, Switch } from '@gojona/ui';
import {
  Archive,
  Check,
  Copy,
  HardDrive,
  Play,
  RefreshCw,
  RotateCw,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  Wifi,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import {
  buildSshCommand,
  formatBytes,
  formatUptime,
  PHASE_META,
  splitBytes,
} from '../lib/strataFormat.ts';
import type { ContainerStatusSnapshot, HudStateResult, WebStatusResult } from '../types.ts';
import { PcPairingCard } from './PcPairingCard.tsx';

/** Props accepted by `OverviewTab`. */
export interface OverviewTabProps {
  container: ContainerStatusSnapshot;
  hudState: HudStateResult;
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  busyAction: string | null;
  onLifecycle: (action: 'start' | 'stop' | 'restart') => Promise<void>;
  onMaintenance: (op: 'dns' | 'compact' | 'fsck' | 'backup') => Promise<void>;
  onUpdateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
  onToggleWebLan: (lanEnabled: boolean) => Promise<void>;
  onRotateWebToken: () => Promise<void>;
  onCopySsh: (sshCommand: string) => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Primary overview tab displaying container status, lifecycle controls, SSH copy, PC pairing, HUD, and maintenance actions. */
export function OverviewTab({
  container,
  hudState,
  webStatus,
  pairingToken,
  busyAction,
  onLifecycle,
  onMaintenance,
  onUpdateHud,
  onToggleWebLan,
  onRotateWebToken,
  onCopySsh,
  onCopyText,
}: OverviewTabProps) {
  const [copied, setCopied] = useState(false);
  const copyTimerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    },
    [],
  );

  const phaseInfo = PHASE_META[container.phase];
  const isRunning = container.phase === 'running';
  const busy = busyAction !== null;
  const sshCommand = buildSshCommand(container);
  const allocated = splitBytes(container.disk.imageAllocatedBytes);

  const rootfsPercent =
    container.disk.rootfsUsedBytes != null &&
    container.disk.rootfsTotalBytes &&
    container.disk.rootfsTotalBytes > 0
      ? `${((container.disk.rootfsUsedBytes / container.disk.rootfsTotalBytes) * 100).toFixed(1)}%`
      : '未挂载';

  const handleCopy = async () => {
    await onCopySsh(sshCommand);
    setCopied(true);
    if (copyTimerRef.current !== null) {
      window.clearTimeout(copyTimerRef.current);
    }
    copyTimerRef.current = window.setTimeout(() => {
      copyTimerRef.current = null;
      setCopied(false);
    }, 1500);
  };

  return (
    <div className="flex flex-col gap-2.5">
      <Card className="flex flex-col gap-3 p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="truncate text-sm font-semibold text-[var(--text-primary)]">
                {container.hostname}
              </span>
              <Badge className="shrink-0" variant={phaseInfo.variant} dot pulse={isRunning}>
                {phaseInfo.label}
              </Badge>
            </div>
            <p className="strata-meta-line mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 font-mono text-[11px] leading-snug text-[var(--text-secondary)]">
              {isRunning ? (
                <>
                  <span>{`PID ${container.nsPid ?? '-'}`}</span>
                  <span>·</span>
                  <span>{`运行 ${formatUptime(container.uptimeMs)}`}</span>
                  <span>·</span>
                  <span className="break-all">{container.loopDevices[0] ?? '无 loop'}</span>
                </>
              ) : (
                <span className="break-all">{`镜像 ${container.imagePath}`}</span>
              )}
            </p>
          </div>
        </div>

        {container.lastError ? (
          <div className="rounded-[var(--radius-control)] border border-[color-mix(in_oklab,var(--danger)_35%,transparent)] bg-[color-mix(in_oklab,var(--danger)_10%,transparent)] px-2.5 py-1.5 text-[11px] text-[var(--danger)]">
            最近错误：{container.lastError}
          </div>
        ) : null}

        <div className="grid grid-cols-3 gap-1.5">
          <Button
            size="sm"
            variant={isRunning ? 'secondary' : 'primary'}
            disabled={busy || isRunning}
            aria-label="启动容器"
            onClick={() => void onLifecycle('start')}
          >
            <Play className="size-3.5" />
            启动
          </Button>
          <Button
            size="sm"
            variant="danger"
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
            disabled={busy}
            aria-label="重启容器"
            onClick={() => void onLifecycle('restart')}
          >
            <RotateCw className="size-3.5" />
            重启
          </Button>
        </div>
      </Card>

      <div className="grid grid-cols-2 gap-2">
        <MetricTile
          label="稀疏镜像"
          value={allocated.value}
          unit={allocated.unit}
          hint={`上限 ${formatBytes(container.disk.imageVirtualBytes)}`}
          statusLabel={rootfsPercent}
          statusVariant={container.mounted ? 'success' : 'neutral'}
          icon={<HardDrive className="size-3.5" />}
        />
        <MetricTile
          label="SSH 服务"
          value={`:${container.sshPort}`}
          hint={container.primaryIpv4}
          statusLabel={container.hostBridge ? '桥接' : '隔离'}
          statusVariant={container.hostBridge ? 'accent' : 'neutral'}
          icon={<Terminal className="size-3.5" />}
        />
      </div>

      <Card className="flex flex-col gap-2 p-3">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">
            SSH 快捷连接
          </span>
          <Button
            className="shrink-0"
            size="sm"
            variant="pill"
            aria-label="复制 SSH 连接命令"
            onClick={() => void handleCopy()}
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            {copied ? '已复制' : '复制命令'}
          </Button>
        </div>
        <div className="truncate rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5 font-mono text-[11px] text-[var(--text-primary)]">
          {sshCommand}
        </div>
      </Card>

      <PcPairingCard
        webStatus={webStatus}
        pairingToken={pairingToken}
        busy={busy}
        onToggleLan={onToggleWebLan}
        onRotateToken={onRotateWebToken}
        onCopyText={onCopyText}
      />

      <Card className="flex flex-col gap-2.5 p-3">
        <SettingRow
          variant="plain"
          title="原生状态 HUD"
          description="零 WebView 矢量状态条，实时显示容器与任务摘要"
          control={
            <Switch
              checked={hudState.enabled}
              onCheckedChange={(checked) => void onUpdateHud({ enabled: checked })}
              aria-label="原生状态 HUD"
            />
          }
        />
        <div className="pochita-divider" />
        <SettingRow
          variant="plain"
          title="HUD 点击穿透"
          description="开启后浮窗忽略触摸事件"
          control={
            <Switch
              checked={hudState.clickThrough}
              disabled={!hudState.enabled}
              onCheckedChange={(checked) => void onUpdateHud({ clickThrough: checked })}
              aria-label="HUD 点击穿透"
            />
          }
        />
      </Card>

      <Card className="flex flex-col gap-2 p-3">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">维护操作</span>
          <span className="shrink-0 text-[11px] text-[var(--text-secondary)]">零残留安全编排</span>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || !container.mounted}
            aria-label="同步 DNS 与宿主机桥接"
            onClick={() => void onMaintenance('dns')}
          >
            <Wifi className="size-3.5" />
            同步 DNS
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            aria-label="执行 FITRIM 空间回收"
            onClick={() => void onMaintenance('compact')}
          >
            <Sparkles className="size-3.5" />
            回收空间
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            aria-label="停机并执行 e2fsck 磁盘检查"
            onClick={() => void onMaintenance('fsck')}
          >
            <ShieldCheck className="size-3.5" />
            检查磁盘
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            aria-label="后台压缩备份镜像"
            onClick={() => void onMaintenance('backup')}
          >
            {busyAction === 'backup' ? (
              <RefreshCw className="size-3.5 animate-spin" />
            ) : (
              <Archive className="size-3.5" />
            )}
            后台备份
          </Button>
        </div>
      </Card>
    </div>
  );
}
