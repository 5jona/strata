import { Badge, Button, Card, cn, ProgressBar } from '@gojona/ui';
import {
  Box,
  ChevronDown,
  ChevronUp,
  Code2,
  Copy,
  Download,
  HardDrive,
  Maximize2,
  Play,
  RotateCw,
  Settings,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  Trash2,
} from 'lucide-react';
import { formatBytes, formatUptime, phaseBadgeMeta } from '../lib/format.ts';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';

export interface ContainerCardProps {
  container: ContainerStatusSnapshot;
  serviceCount: number;
  runningServiceCount: number;
  expanded: boolean;
  busy: boolean;
  onToggleExpand: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onOpenTerminal: () => void;
  onOpenVsCode: () => void;
  onCopySsh: () => void;
  onCompact: () => void;
  onFsck: () => void;
  onResize: () => void;
  onClone: () => void;
  onExport: (format: 'tar.gz' | 'img.gz') => void;
  onConfig: () => void;
  onDelete: () => void;
}

/**
 * Progressive-disclosure container card inspired by `owu/wsl-dashboard`:
 * compact primary row for 1-click daily actions + smooth expandable tray for maintenance.
 */
export function ContainerCard({
  container,
  serviceCount,
  runningServiceCount,
  expanded,
  busy,
  onToggleExpand,
  onStart,
  onStop,
  onRestart,
  onOpenTerminal,
  onOpenVsCode,
  onCopySsh,
  onCompact,
  onFsck,
  onResize,
  onClone,
  onExport,
  onConfig,
  onDelete,
}: ContainerCardProps) {
  const phase = phaseBadgeMeta(container.phase);
  const running = container.phase === 'running';
  const diskRatio =
    container.disk.imageVirtualBytes > 0
      ? Math.min(
          100,
          Math.round((container.disk.imageAllocatedBytes / container.disk.imageVirtualBytes) * 100),
        )
      : 0;

  return (
    <Card className="overflow-hidden border-[var(--border-hairline)] p-0">
      {/* Compact Primary Row */}
      <div className="flex flex-wrap items-center gap-3 px-4 py-3">
        <div className="flex min-w-[200px] flex-1 items-center gap-3">
          <div
            className={cn(
              'flex size-10 shrink-0 items-center justify-center rounded-xl border',
              running
                ? 'border-[var(--success)]/35 bg-[var(--success)]/12 text-[var(--success)]'
                : 'border-[var(--border-hairline)] bg-[var(--surface-elevated)] text-[var(--text-secondary)]',
            )}
          >
            <Box className="size-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="truncate text-sm font-semibold text-[var(--text-primary)]">
                {container.id}
              </span>
              <Badge variant={phase.variant}>{phase.label}</Badge>
              {container.isDefault && <Badge variant="accent">默认</Badge>}
              {container.autoStart && <Badge variant="neutral">开机自启</Badge>}
              {serviceCount > 0 && (
                <Badge variant={runningServiceCount > 0 ? 'success' : 'neutral'}>
                  服务 {runningServiceCount}/{serviceCount}
                </Badge>
              )}
            </div>
            <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
              <span>{container.hostname}</span>
              <span>SSH :{container.sshPort}</span>
              <span>运行 {formatUptime(container.uptimeMs)}</span>
              {container.nsPid && <span>nsPID {container.nsPid}</span>}
            </div>
          </div>
        </div>

        {/* Disk Usage Bar */}
        <div className="w-44 shrink-0">
          <div className="mb-1 flex items-center justify-between text-[11px] text-[var(--text-secondary)]">
            <span className="flex items-center gap-1">
              <HardDrive className="size-3" />
              镜像占用
            </span>
            <span className="font-mono">
              {formatBytes(container.disk.imageAllocatedBytes)} /{' '}
              {formatBytes(container.disk.imageVirtualBytes)}
            </span>
          </div>
          <ProgressBar
            size="sm"
            value={diskRatio}
            tone={diskRatio > 85 ? 'warning' : 'accent'}
            ariaLabel={`${container.id} 磁盘占用 ${diskRatio}%`}
          />
        </div>

        {/* Primary Quick Actions */}
        <div className="flex shrink-0 items-center gap-1.5">
          {running ? (
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
            disabled={!running}
            title="打开内置终端标签页"
            aria-label={`打开 ${container.id} 终端`}
            onClick={onOpenTerminal}
          >
            <Terminal className="size-3.5" />
          </Button>
          <Button
            size="icon-sm"
            variant="secondary"
            disabled={!running || !container.sshEnabled}
            title="在 VS Code Remote-SSH 中打开"
            aria-label={`在 VS Code 中打开 ${container.id}`}
            onClick={onOpenVsCode}
          >
            <Code2 className="size-3.5" />
          </Button>
          <Button
            size="icon-sm"
            variant="secondary"
            title="复制 SSH 命令"
            aria-label={`复制 ${container.id} SSH 命令`}
            onClick={onCopySsh}
          >
            <Copy className="size-3.5" />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-expanded={expanded}
            aria-label={expanded ? '收起高级操作栏' : '展开高级操作栏'}
            onClick={onToggleExpand}
          >
            {expanded ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
          </Button>
        </div>
      </div>

      {/* Expandable Secondary Tray */}
      {expanded && (
        <div className="strata-tray-expand flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-hairline)] bg-[var(--surface-elevated)]/75 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-1.5">
            <Button size="sm" variant="ghost" disabled={busy} onClick={onRestart}>
              <RotateCw className="size-3.5" />
              重启
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onCompact}>
              <Sparkles className="size-3.5" />
              回收空间 (FITRIM)
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onFsck}>
              <ShieldCheck className="size-3.5" />
              磁盘检查 (fsck)
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onResize}>
              <Maximize2 className="size-3.5" />
              扩容镜像
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onClone}>
              <Copy className="size-3.5" />
              克隆容器
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => onExport('tar.gz')}>
              <Download className="size-3.5" />
              导出 .tar.gz
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => onExport('img.gz')}>
              <Download className="size-3.5" />
              导出 .img.gz
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onConfig}>
              <Settings className="size-3.5" />
              高级配置
            </Button>
          </div>
          <Button size="sm" variant="danger" disabled={busy} onClick={onDelete}>
            <Trash2 className="size-3.5" />
            删除容器
          </Button>
        </div>
      )}
    </Card>
  );
}
