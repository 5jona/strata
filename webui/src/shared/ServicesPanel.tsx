import { Badge, Button, Card, cn, Switch } from '@gojona/ui';
import {
  ChevronDown,
  ChevronUp,
  Copy,
  FileText,
  Play,
  RefreshCw,
  RotateCw,
  ServerCog,
  Square,
} from 'lucide-react';
import { useState } from 'react';
import type { ContainerServiceRuntime, ServiceStatus } from '../types.ts';
import { formatUptime, SERVICE_STATUS_META } from './strataFormat.ts';

export type ServiceControlAction = 'start' | 'stop' | 'restart';

/** Props accepted by `ServicesPanel`. */
export interface ServicesPanelProps {
  containerId: string;
  containerRunning: boolean;
  services: readonly ContainerServiceRuntime[];
  serviceLogs?: Readonly<Record<string, readonly string[]>>;
  busyKey: string | null;
  compact?: boolean;
  onRefresh?: () => Promise<void>;
  onAction: (serviceId: string, action: ServiceControlAction) => Promise<void>;
  onToggleAutoStart?: (serviceId: string, autoStart: boolean) => Promise<void>;
  onLoadLogs?: (serviceId: string) => Promise<void>;
  onCopyText?: (text: string, label: string) => Promise<void>;
}

const STATUS_DOT_CLASSES: Record<ServiceStatus, string> = {
  running: 'bg-[var(--success)]',
  stopped: 'bg-[var(--text-tertiary)]',
  crashed: 'bg-[var(--danger)]',
  'circuit-open': 'bg-[var(--warning)]',
};

/**
 * Shared managed background services panel.
 * Renders a compact zero-input list in SideVeil (`compact`) and full controls with auto-start + logs in KSU.
 */
export function ServicesPanel({
  containerId,
  containerRunning,
  services,
  serviceLogs = {},
  busyKey,
  compact = false,
  onRefresh,
  onAction,
  onToggleAutoStart,
  onLoadLogs,
  onCopyText,
}: ServicesPanelProps) {
  const [expandedLogId, setExpandedLogId] = useState<string | null>(null);
  const runningCount = services.filter((s) => s.status === 'running').length;

  const handleToggleLogs = async (serviceId: string) => {
    if (!onLoadLogs) return;
    if (expandedLogId === serviceId) {
      setExpandedLogId(null);
      return;
    }
    setExpandedLogId(serviceId);
    await onLoadLogs(serviceId);
  };

  if (compact) {
    return (
      <Card className="flex flex-col gap-2 p-3">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <ServerCog className="size-3.5 shrink-0 text-[var(--accent)]" />
            <span className="truncate text-xs font-medium text-[var(--text-primary)]">
              托管服务
            </span>
          </div>
          <Badge className="shrink-0" variant="neutral">
            {runningCount}/{services.length}
          </Badge>
        </div>

        {services.length === 0 ? (
          <p className="py-1 text-[11px] text-[var(--text-secondary)]">暂无托管后台服务</p>
        ) : (
          <div className="flex flex-col divide-y divide-[var(--border-hairline)]">
            {services.map((svc) => {
              const isRunning = svc.status === 'running';
              const isBusy = busyKey !== null;
              return (
                <div
                  key={svc.id}
                  className="flex items-center justify-between gap-2 py-2 first:pt-0.5 last:pb-0.5"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      aria-hidden="true"
                      className={cn('size-2 shrink-0 rounded-full', STATUS_DOT_CLASSES[svc.status])}
                    />
                    <span className="truncate text-xs font-medium text-[var(--text-primary)]">
                      {svc.name}
                    </span>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      size="icon-sm"
                      variant={isRunning ? 'secondary' : 'primary'}
                      disabled={isBusy || (!isRunning && !containerRunning)}
                      aria-label={`${isRunning ? '停止' : '启动'}服务 ${svc.name}`}
                      onClick={() => void onAction(svc.id, isRunning ? 'stop' : 'start')}
                    >
                      {isRunning ? <Square className="size-3" /> : <Play className="size-3" />}
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      disabled={isBusy || !containerRunning}
                      aria-label={`重启服务 ${svc.name}`}
                      onClick={() => void onAction(svc.id, 'restart')}
                    >
                      <RotateCw className="size-3" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-2.5">
      <Card className="flex items-center justify-between gap-2 p-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <ServerCog className="size-3.5 shrink-0 text-[var(--accent)]" />
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">
            托管后台服务 · {containerId}
          </span>
          <Badge className="shrink-0" variant="neutral">
            {runningCount}/{services.length}
          </Badge>
        </div>
        {onRefresh ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="刷新托管服务列表"
            onClick={() => void onRefresh()}
          >
            <RefreshCw className="size-3.5" />
          </Button>
        ) : null}
      </Card>

      {services.length === 0 ? (
        <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
          当前容器未配置托管后台服务（可在 Strata Desktop 中添加自启服务）。
        </Card>
      ) : (
        services.map((svc) => {
          const meta = SERVICE_STATUS_META[svc.status];
          const isRunning = svc.status === 'running';
          const isBusy = busyKey !== null;
          const logsOpen = expandedLogId === svc.id;
          const lines = serviceLogs[svc.id] ?? [];

          return (
            <Card key={svc.id} className="flex flex-col gap-2.5 p-3">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="truncate text-xs font-semibold text-[var(--text-primary)]">
                      {svc.name}
                    </span>
                    <Badge className="shrink-0" variant={meta.variant} dot pulse={isRunning}>
                      {meta.label}
                    </Badge>
                  </div>
                  <p className="mt-0.5 flex flex-wrap items-center gap-x-1.5 font-mono text-[10px] text-[var(--text-secondary)]">
                    <span>{svc.id}</span>
                    {svc.pid ? <span>· PID {svc.pid}</span> : null}
                    <span>· {formatUptime(svc.uptimeMs)}</span>
                    {svc.restartCount > 0 ? <span>· 重启 {svc.restartCount}</span> : null}
                  </p>
                  {svc.lastError ? (
                    <p className="mt-1 truncate text-[10px] text-[var(--danger)]">
                      错误：{svc.lastError}
                    </p>
                  ) : null}
                </div>

                {onToggleAutoStart ? (
                  <div className="flex shrink-0 items-center gap-1.5">
                    <span className="text-[11px] text-[var(--text-secondary)]">自启</span>
                    <Switch
                      checked={svc.autoStart}
                      disabled={isBusy}
                      aria-label={`切换服务 ${svc.name} 开机自启`}
                      onCheckedChange={(checked) => void onToggleAutoStart(svc.id, checked)}
                    />
                  </div>
                ) : null}
              </div>

              <div className="grid grid-cols-4 gap-1.5">
                <Button
                  size="sm"
                  variant={isRunning ? 'secondary' : 'primary'}
                  disabled={isBusy || (!isRunning && !containerRunning)}
                  aria-label={`${isRunning ? '停止' : '启动'}服务 ${svc.name}`}
                  onClick={() => void onAction(svc.id, isRunning ? 'stop' : 'start')}
                >
                  {isRunning ? <Square className="size-3" /> : <Play className="size-3" />}
                  {isRunning ? '停止' : '启动'}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={isBusy || !containerRunning}
                  aria-label={`重启服务 ${svc.name}`}
                  onClick={() => void onAction(svc.id, 'restart')}
                >
                  <RotateCw className="size-3" />
                  重启
                </Button>
                <Button
                  className="col-span-2"
                  size="sm"
                  variant={logsOpen ? 'pill' : 'ghost'}
                  aria-expanded={logsOpen}
                  aria-label={`展开服务 ${svc.name} 日志`}
                  onClick={() => void handleToggleLogs(svc.id)}
                >
                  <FileText className="size-3" />
                  日志
                  {logsOpen ? <ChevronUp className="size-3" /> : <ChevronDown className="size-3" />}
                </Button>
              </div>

              {logsOpen ? (
                <div className="flex flex-col gap-1.5 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-2">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                      最近输出 ({lines.length} 行)
                    </span>
                    <div className="flex items-center gap-1">
                      {onLoadLogs ? (
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`刷新 ${svc.name} 日志`}
                          onClick={() => void onLoadLogs(svc.id)}
                        >
                          <RefreshCw className="size-3" />
                        </Button>
                      ) : null}
                      {onCopyText ? (
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`复制 ${svc.name} 日志`}
                          disabled={lines.length === 0}
                          onClick={() => void onCopyText(lines.join('\n'), `${svc.name} 日志`)}
                        >
                          <Copy className="size-3" />
                        </Button>
                      ) : null}
                    </div>
                  </div>
                  <pre className="max-h-36 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[10px] leading-relaxed text-[var(--text-primary)]">
                    {lines.length > 0 ? lines.join('\n') : '暂无日志输出'}
                  </pre>
                </div>
              ) : null}
            </Card>
          );
        })
      )}
    </div>
  );
}
