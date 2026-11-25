import { MetricTile } from '@gojona/ui';
import { Box, HardDrive, ServerCog, Shield } from 'lucide-react';
import { formatBytes, formatUptime } from '../lib/format.ts';
import type { ContainerStatusSnapshot, DaemonSummarySnapshot } from '../types/protocol.ts';

export interface ContainersKpiStripProps {
  containers: ContainerStatusSnapshot[];
  runningServiceCount: number;
  daemonStatus: DaemonSummarySnapshot | null;
}

/** Top 4-tile KPI summary strip for the desktop Containers view. */
export function ContainersKpiStrip({
  containers,
  runningServiceCount,
  daemonStatus,
}: ContainersKpiStripProps) {
  const totalAllocated = containers.reduce((sum, c) => sum + c.disk.imageAllocatedBytes, 0);
  const runningCount = containers.filter((c) => c.phase === 'running').length;

  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <MetricTile
        label="运行中容器"
        value={`${runningCount}/${containers.length}`}
        icon={<Box className="size-4 text-[var(--accent)]" />}
        statusLabel={runningCount > 0 ? '活跃' : '空闲'}
        statusVariant={runningCount > 0 ? 'success' : 'neutral'}
      />
      <MetricTile
        label="托管后台服务"
        value={runningServiceCount}
        unit="个运行中"
        icon={<ServerCog className="size-4 text-[var(--accent)]" />}
      />
      <MetricTile
        label="磁盘实际占用"
        value={formatBytes(totalAllocated)}
        icon={<HardDrive className="size-4 text-[var(--accent)]" />}
      />
      <MetricTile
        label="stratad 运行时长"
        value={daemonStatus ? formatUptime(daemonStatus.uptimeMs) : '—'}
        hint={daemonStatus ? `PID ${daemonStatus.pid}` : undefined}
        icon={<Shield className="size-4 text-[var(--accent)]" />}
      />
    </div>
  );
}
