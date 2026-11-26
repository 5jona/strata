import { Badge, Button, Card, MetricTile, StatusBanner } from '@gojona/ui';
import { Copy, HardDrive, Play, RotateCw, Square } from 'lucide-react';
import {
  buildSshCommand,
  formatBytes,
  formatUptime,
  PHASE_META,
  splitBytes,
} from '../shared/strataFormat.ts';
import type { ContainerStatusSnapshot } from '../types.ts';
import { KsuContainerFleetCard } from './KsuContainerFleetCard.tsx';
import { KsuMaintenanceCard } from './KsuMaintenanceCard.tsx';
import type { CreateContainerOptions, KsuMaintenanceOp } from './useKsuActions.ts';

/** Props accepted by `KsuOverviewTab`. */
export interface KsuOverviewTabProps {
  container: ContainerStatusSnapshot;
  containers: readonly ContainerStatusSnapshot[];
  selectedId: string | null;
  busyKey: string | null;
  onSelectContainer: (containerId: string) => void;
  onLifecycle: (action: 'start' | 'stop' | 'restart') => Promise<void>;
  onMaintenance: (op: KsuMaintenanceOp) => Promise<void>;
  onSetDefault: (containerId: string) => Promise<void>;
  onDeleteContainer: (containerId: string, deleteImage: boolean) => Promise<void>;
  onCreateContainer: (containerId: string, options?: CreateContainerOptions) => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
  onOpenSettings?: () => void;
}

/** Tab 1: container overview, metrics, SSH copy, missing-image recovery, and fleet management. */
export function KsuOverviewTab({
  container,
  containers,
  selectedId,
  busyKey,
  onSelectContainer,
  onLifecycle,
  onMaintenance,
  onSetDefault,
  onDeleteContainer,
  onCreateContainer,
  onCopyText,
  onOpenSettings,
}: KsuOverviewTabProps) {
  const phaseMeta = PHASE_META[container.phase];
  const isRunning = container.phase === 'running';
  const busy = busyKey !== null;
  const imageMissing = !container.disk.imageExists;
  const sshCommand = buildSshCommand(container);
  const createCmd = `strata rpc container.create '{"id":"${container.id}","distroId":"debian-12","sizeMb":4096}'`;
  const allocated = splitBytes(container.disk.imageAllocatedBytes);
  const rootfsUsed = splitBytes(container.disk.rootfsUsedBytes ?? 0);
  const hostPrefix =
    container.hostname && container.hostname !== container.id ? `${container.hostname} · ` : '';

  return (
    <div className="flex flex-col gap-3">
      {imageMissing ? (
        <StatusBanner
          tone="warning"
          title="尚未初始化系统镜像"
          description={`镜像路径 ${container.imagePath} 尚未创建。可前往「系统设置 → 电脑端配对连接」使用 Strata Desktop 创建，或在 Root 终端执行：${createCmd}`}
          action={
            <div className="flex flex-wrap items-center gap-1.5">
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void onCopyText(createCmd, '初始化命令')}
              >
                <Copy className="size-3.5" />
                复制初始化命令
              </Button>
              {onOpenSettings ? (
                <Button size="sm" variant="primary" onClick={onOpenSettings}>
                  去开启 PC 配对
                </Button>
              ) : null}
            </div>
          }
        />
      ) : null}

      <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
        <div className="flex flex-col gap-3 md:col-span-5">
          <Card className="flex flex-col gap-3 p-4">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="truncate text-sm font-semibold text-[var(--text-primary)]">
                    {container.id}
                  </span>
                  {container.isDefault ? <Badge variant="accent">默认</Badge> : null}
                  <Badge variant={phaseMeta.variant} dot pulse={isRunning}>
                    {phaseMeta.label}
                  </Badge>
                </div>
                <p className="mt-1 truncate font-mono text-[11px] text-[var(--text-secondary)]">
                  {isRunning
                    ? `${hostPrefix}PID ${container.nsPid ?? '-'} · 运行 ${formatUptime(container.uptimeMs)}`
                    : `${hostPrefix}${container.imagePath}`}
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
                SSH 终端连接（仅公钥免密）
              </span>
              <p className="truncate font-mono text-xs text-[var(--text-primary)]">
                {container.sshEnabled ? sshCommand : 'SSH 未启用（可在「配置」中开启）'}
              </p>
            </div>
            <Button
              className="shrink-0"
              size="sm"
              variant="pill"
              disabled={!container.sshEnabled}
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
            label="SSH 服务"
            value={container.sshEnabled ? `:${container.sshPort}` : '未启用'}
            hint={container.sshEnabled ? container.primaryIpv4 : '纯容器模式'}
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
              container.hostRuntime ? '宿主机直通' : container.hostBridge ? '桥接模式' : '标准隔离'
            }
            hint={container.mountDir}
            statusLabel={container.mounted ? '已挂载' : '未挂载'}
            statusVariant={container.mounted ? 'success' : 'neutral'}
          />
        </div>
      </div>

      <KsuMaintenanceCard
        container={container}
        busyKey={busyKey}
        onMaintenance={onMaintenance}
        onSetDefault={onSetDefault}
        onDeleteContainer={onDeleteContainer}
      />

      <KsuContainerFleetCard
        containers={containers}
        selectedId={selectedId}
        busyKey={busyKey}
        onSelectContainer={onSelectContainer}
        onSetDefault={onSetDefault}
        onCreateContainer={onCreateContainer}
        onCopyText={onCopyText}
      />
    </div>
  );
}
