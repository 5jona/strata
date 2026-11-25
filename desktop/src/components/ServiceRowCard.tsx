import { Badge, Button, Card, Switch } from '@gojona/ui';
import { FileText, Pencil, Play, RotateCw, Square, Trash2 } from 'lucide-react';
import { formatUptime, serviceStatusMeta } from '../lib/format.ts';
import type { ContainerServiceRuntime, ContainerServiceSpec } from '../types/protocol.ts';

export interface ServiceRowCardProps {
  service: ContainerServiceRuntime;
  spec?: ContainerServiceSpec;
  busy: boolean;
  logActive: boolean;
  onToggleAutoStart: (next: boolean) => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onViewLogs: () => void;
  onEdit: () => void;
  onDelete: () => void;
}

/**
 * Single managed background service row card with lifecycle controls and log trigger.
 */
export function ServiceRowCard({
  service,
  spec,
  busy,
  logActive,
  onToggleAutoStart,
  onStart,
  onStop,
  onRestart,
  onViewLogs,
  onEdit,
  onDelete,
}: ServiceRowCardProps) {
  const meta = serviceStatusMeta(service.status);
  const isRunning = service.status === 'running';

  return (
    <Card className="flex flex-wrap items-center justify-between gap-3 p-3.5">
      <div className="min-w-[220px] flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--text-primary)]">{service.name}</span>
          <span className="font-mono text-xs text-[var(--text-secondary)]">({service.id})</span>
          <Badge variant={meta.variant}>{meta.label}</Badge>
          {service.pid && <Badge variant="neutral">PID {service.pid}</Badge>}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
          {spec && <span className="truncate">{spec.command.join(' ')}</span>}
          <span>运行 {formatUptime(service.uptimeMs)}</span>
          <span>重启 {service.restartCount} 次</span>
          {service.lastError && (
            <span className="text-[var(--danger)]">错误：{service.lastError}</span>
          )}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
          <span>开机自启</span>
          <Switch
            checked={service.autoStart}
            disabled={busy}
            aria-label={`开机自启 ${service.name}`}
            onCheckedChange={onToggleAutoStart}
          />
        </div>
        {isRunning ? (
          <Button size="sm" variant="secondary" disabled={busy} onClick={onStop}>
            <Square className="size-3.5" />
            停止
          </Button>
        ) : (
          <Button size="sm" variant="primary" disabled={busy} onClick={onStart}>
            <Play className="size-3.5" />
            启动
          </Button>
        )}
        <Button
          size="icon-sm"
          variant="secondary"
          title="重启服务"
          aria-label={`重启 ${service.name}`}
          disabled={busy}
          onClick={onRestart}
        >
          <RotateCw className="size-3.5" />
        </Button>
        <Button
          size="icon-sm"
          variant={logActive ? 'pill' : 'secondary'}
          title="查看日志"
          aria-label={`查看 ${service.name} 日志`}
          onClick={onViewLogs}
        >
          <FileText className="size-3.5" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          title="编辑服务"
          aria-label={`编辑 ${service.name}`}
          onClick={onEdit}
        >
          <Pencil className="size-3.5" />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          title="删除服务"
          aria-label={`删除 ${service.name}`}
          onClick={onDelete}
        >
          <Trash2 className="size-3.5 text-[var(--danger)]" />
        </Button>
      </div>
    </Card>
  );
}
