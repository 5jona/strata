import { Badge, Button, Card, MetricTile, StatusBanner } from '@gojona/ui';
import {
  Archive,
  Copy,
  HardDrive,
  Play,
  RefreshCw,
  RotateCw,
  ShieldCheck,
  Sparkles,
  Square,
  Wifi,
} from 'lucide-react';
import { useState } from 'react';
import {
  buildSshCommand,
  formatBytes,
  formatUptime,
  PHASE_META,
  splitBytes,
} from '../shared/strataFormat.ts';
import type { ContainerStatusSnapshot } from '../types.ts';
import type { KsuMaintenanceOp } from './useKsuActions.ts';

/** Props accepted by `KsuOverviewTab`. */
export interface KsuOverviewTabProps {
  container: ContainerStatusSnapshot;
  busyKey: string | null;
  onLifecycle: (action: 'start' | 'stop' | 'restart') => Promise<void>;
  onMaintenance: (op: KsuMaintenanceOp) => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Tab 1: container overview, metrics, SSH copy, missing-image recovery, and safe maintenance. */
export function KsuOverviewTab({
  container,
  busyKey,
  onLifecycle,
  onMaintenance,
  onCopyText,
}: KsuOverviewTabProps) {
  const [confirmFsck, setConfirmFsck] = useState(false);

  const phaseMeta = PHASE_META[container.phase];
  const isRunning = container.phase === 'running';
  const busy = busyKey !== null;
  const imageMissing = !container.disk.imageExists;
  const sshCommand = buildSshCommand(container);
  const allocated = splitBytes(container.disk.imageAllocatedBytes);
  const rootfsUsed = splitBytes(container.disk.rootfsUsedBytes ?? 0);

  return (
    <div className="flex flex-col gap-3">
      {imageMissing ? (
        <StatusBanner
          tone="warning"
          title="未检测到容器 rootfs 镜像"
          description={`镜像路径 ${container.imagePath} 尚未初始化。请通过 PC 端 Strata Desktop 的在线发行版目录创建，或在 Root 终端执行 strata create ${container.id} 初始化容器。`}
        />
      ) : null}

      <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
        <div className="flex flex-col gap-3 md:col-span-5">
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-semibold text-[var(--text-primary)]">
                    {container.hostname}
                  </span>
                  <Badge variant={phaseMeta.variant} dot pulse={isRunning}>
                    {phaseMeta.label}
                  </Badge>
                </div>
                <p className="mt-1 font-mono text-[11px] text-[var(--text-secondary)]">
                  {isRunning
                    ? `PID ${container.nsPid ?? '-'} · 运行 ${formatUptime(container.uptimeMs)}`
                    : container.imagePath}
                </p>
              </div>
            </div>

            {container.lastError ? (
              <StatusBanner tone="danger" title="最近错误" description={container.lastError} />
            ) : null}

            <div className="grid grid-cols-3 gap-2">
              <Button
                size="sm"
                variant="primary"
                disabled={busy || isRunning || imageMissing}
                onClick={() => void onLifecycle('start')}
              >
                <Play className="size-3.5" />
                启动
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={busy || !isRunning}
                onClick={() => void onLifecycle('stop')}
              >
                <Square className="size-3.5" />
                停止
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={busy || imageMissing}
                onClick={() => void onLifecycle('restart')}
              >
                <RotateCw className="size-3.5" />
                重启
              </Button>
            </div>
          </Card>

          <Card className="flex items-center justify-between gap-2 p-3">
            <div className="min-w-0">
              <span className="text-[11px] font-medium text-[var(--text-secondary)]">
                Dropbear SSH 连接
              </span>
              <p className="truncate font-mono text-xs text-[var(--text-primary)]">{sshCommand}</p>
            </div>
            <Button
              className="shrink-0"
              size="sm"
              variant="pill"
              onClick={() => void onCopyText(sshCommand, 'SSH 命令')}
            >
              <Copy className="size-3.5" />
              复制
            </Button>
          </Card>
        </div>

        <div className="grid grid-cols-2 gap-3 md:col-span-7">
          <MetricTile
            label="镜像占用"
            value={allocated.value}
            unit={allocated.unit}
            hint={`上限 ${formatBytes(container.disk.imageVirtualBytes)}`}
            icon={<HardDrive className="size-3.5" />}
          />
          <MetricTile
            label="Dropbear SSH"
            value={`:${container.sshPort}`}
            hint={container.primaryIpv4}
            statusLabel={container.sshEnabled ? '启用' : '关闭'}
            statusVariant={container.sshEnabled ? 'success' : 'neutral'}
          />
          <MetricTile
            label="根文件系统"
            value={rootfsUsed.value}
            unit={rootfsUsed.unit}
            hint={`可用 ${formatBytes(container.disk.rootfsAvailableBytes ?? 0)}`}
          />
          <MetricTile
            label="运行模式"
            value={
              container.hostRuntime ? 'HostRuntime' : container.hostBridge ? 'Bridge' : 'Isolated'
            }
            hint={container.mountDir}
            statusLabel={container.mounted ? '已挂载' : '未挂载'}
            statusVariant={container.mounted ? 'success' : 'neutral'}
          />
        </div>
      </div>

      <Card className="flex flex-col gap-2.5 p-4">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-[var(--text-primary)]">零残留维护操作</span>
          <span className="text-[11px] text-[var(--text-secondary)]">
            串行持锁执行 · 安全卸载防残留
          </span>
        </div>

        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || !container.mounted}
            onClick={() => void onMaintenance('dns')}
          >
            <Wifi className="size-3.5" />
            同步 DNS
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || imageMissing}
            onClick={() => void onMaintenance('compact')}
          >
            <Sparkles className="size-3.5" />
            回收空间 compact
          </Button>
          <Button
            size="sm"
            variant={confirmFsck ? 'danger' : 'secondary'}
            disabled={busy || imageMissing}
            onClick={() => setConfirmFsck((prev) => !prev)}
          >
            <ShieldCheck className="size-3.5" />
            停机检盘 fsck
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || imageMissing}
            onClick={() => void onMaintenance('backup')}
          >
            {busyKey === 'maint:backup' ? (
              <RefreshCw className="size-3.5 animate-spin" />
            ) : (
              <Archive className="size-3.5" />
            )}
            本地镜像备份
          </Button>
        </div>

        {confirmFsck ? (
          <StatusBanner
            tone="warning"
            title="确认执行停机检盘 (e2fsck)？"
            description="该操作会先安全停止当前容器并卸载镜像，再执行 ext4 文件系统检查修复。"
            action={
              <div className="flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="danger"
                  disabled={busy}
                  onClick={() => {
                    setConfirmFsck(false);
                    void onMaintenance('fsck');
                  }}
                >
                  确认检盘
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setConfirmFsck(false)}>
                  取消
                </Button>
              </div>
            }
          />
        ) : null}
      </Card>
    </div>
  );
}
