import { Badge, Button, Card, SegmentedControl } from '@gojona/ui';
import { RefreshCw, Sparkles, Trash2, Wrench } from 'lucide-react';
import { formatBytes } from '../../lib/format.ts';
import type {
  ContainerStatusSnapshot,
  ToolManagerHealth,
  ToolsBootstrapProfile,
  ToolsStatusResult,
} from '../../types/protocol.ts';

function HealthBadge({ label, health }: { label: string; health?: ToolManagerHealth }) {
  return (
    <Badge variant={health?.installed ? 'success' : 'neutral'}>
      {label}: {health?.installed ? health.version || '已就绪' : '未安装'}
    </Badge>
  );
}

export interface ToolsOverviewBarProps {
  containers: ContainerStatusSnapshot[];
  selectedContainer: string;
  profile: ToolsBootstrapProfile;
  status: ToolsStatusResult | null;
  busy: boolean;
  disabled: boolean;
  onSelectContainer: (id: string) => void;
  onSelectProfile: (profile: ToolsBootstrapProfile) => void;
  onBootstrap: () => void;
  onCleanCache: () => void;
  onRefresh: () => void;
}

/** Top overview bar for the container toolchain workbench. */
export function ToolsOverviewBar({
  containers,
  selectedContainer,
  profile,
  status,
  busy,
  disabled,
  onSelectContainer,
  onSelectProfile,
  onBootstrap,
  onCleanCache,
  onRefresh,
}: ToolsOverviewBarProps) {
  return (
    <Card className="flex shrink-0 flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Wrench className="size-4 text-[var(--accent)]" />
          <h1 className="text-base font-semibold text-[var(--text-primary)]">容器工具环境</h1>
          <div className="flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-0.5">
            {containers.map((c) => (
              <Button
                key={c.id}
                size="sm"
                variant={selectedContainer === c.id ? 'pill' : 'ghost'}
                onClick={() => onSelectContainer(c.id)}
              >
                {c.id}
              </Button>
            ))}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl
            size="sm"
            value={profile}
            onValueChange={onSelectProfile}
            options={[
              { value: 'minimal', label: 'Minimal' },
              { value: 'dev', label: 'Dev' },
              { value: 'debug', label: 'Debug' },
            ]}
          />
          <Button size="sm" variant="primary" disabled={busy || disabled} onClick={onBootstrap}>
            <Sparkles className="size-3.5" />
            规范化初始化 (Bootstrap)
          </Button>
          <Button size="sm" variant="secondary" disabled={busy || disabled} onClick={onCleanCache}>
            <Trash2 className="size-3.5" />
            清理缓存
          </Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={onRefresh}>
            <RefreshCw className={`size-3.5 ${busy ? 'animate-spin' : ''}`} />
            刷新
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        <HealthBadge label="mise" health={status?.mise} />
        <HealthBadge label="uv" health={status?.uv} />
        <HealthBadge label="apt" health={status?.apt} />
        <HealthBadge label="host-exec" health={status?.hostExec} />
        <HealthBadge label="strata-debug" health={status?.strataDebug} />
        {status && (
          <Badge variant={status.layoutCompliant ? 'accent' : 'warning'}>
            {status.layoutCompliant ? '4 路径布局就绪' : '布局未初始化'} · 缓存{' '}
            {formatBytes(status.cacheBytes)}
          </Badge>
        )}
      </div>
    </Card>
  );
}
