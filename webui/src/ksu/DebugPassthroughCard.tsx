import { Badge, Button, Card, SettingRow, Switch } from '@gojona/ui';
import { Cpu } from 'lucide-react';
import {
  COMMON_DEBUG_CHAR_DEVS,
  getCustomCharDevs,
  getCustomMounts,
  hasL5DiagMounts,
  toggleCharDev,
  toggleL5DiagMounts,
} from '../shared/passthrough.ts';
import type { BindMount } from '../types.ts';

/** Props accepted by `DebugPassthroughCard`. */
export interface DebugPassthroughCardProps {
  hostRuntime: boolean;
  onHostRuntimeChange: (next: boolean) => void;
  extraCharDevs: readonly string[];
  onExtraCharDevsChange: (next: string[]) => void;
  extraMounts: readonly BindMount[];
  onExtraMountsChange: (next: BindMount[]) => void;
  disabled?: boolean;
}

/** Card controlling opt-in host runtime virtual FS passthrough, L5 read-only mounts, and char devices. */
export function DebugPassthroughCard({
  hostRuntime,
  onHostRuntimeChange,
  extraCharDevs,
  onExtraCharDevsChange,
  extraMounts,
  onExtraMountsChange,
  disabled = false,
}: DebugPassthroughCardProps) {
  const l5Enabled = hasL5DiagMounts(extraMounts);
  const customCharDevs = getCustomCharDevs(extraCharDevs);
  const customMounts = getCustomMounts(extraMounts);

  return (
    <Card className="flex flex-col gap-2.5 p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <Cpu className="size-3.5 shrink-0 text-[var(--accent)]" />
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">
            调试透传与 L5 诊断
          </span>
        </div>
        <Badge className="shrink-0" variant="neutral">
          重启容器生效
        </Badge>
      </div>

      <SettingRow
        variant="plain"
        title="宿主机运行时"
        description="直通宿主机 /proc、/sys、/dev 并启用 FullHostRuntime 完整权限集（重启容器后生效）"
        control={
          <Switch
            checked={hostRuntime}
            disabled={disabled}
            onCheckedChange={onHostRuntimeChange}
            aria-label="宿主机运行时"
          />
        }
      />

      <div className="pochita-divider" />

      <SettingRow
        variant="plain"
        title="L5 内核诊断只读挂载"
        description="只读挂载 /sys/fs/pstore、/sys/kernel/tracing 与 /sys/fs/bpf"
        control={
          <Switch
            checked={l5Enabled}
            disabled={disabled}
            onCheckedChange={(checked) =>
              onExtraMountsChange(toggleL5DiagMounts(extraMounts, checked))
            }
            aria-label="L5 内核诊断只读挂载"
          />
        }
      />

      <div className="pochita-divider" />

      <div className="flex flex-col gap-1.5">
        <span className="text-[11px] font-medium text-[var(--text-secondary)]">
          字符设备透传（/dev/*）
        </span>
        <div className="flex flex-wrap gap-1.5">
          {COMMON_DEBUG_CHAR_DEVS.map((opt) => {
            const active = extraCharDevs.includes(opt.name);
            return (
              <Button
                key={opt.name}
                className="shrink-0"
                size="sm"
                variant={active ? 'pill' : 'secondary'}
                disabled={disabled}
                aria-pressed={active}
                aria-label={`${active ? '取消' : '启用'}字符设备 ${opt.label}（${opt.description}）`}
                onClick={() =>
                  onExtraCharDevsChange(toggleCharDev(extraCharDevs, opt.name, !active))
                }
              >
                <span className="font-mono">{opt.label}</span>
                <span className="text-[10px] opacity-80">{opt.description}</span>
              </Button>
            );
          })}
        </div>

        {customCharDevs.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1 pt-0.5">
            <span className="text-[10px] text-[var(--text-secondary)]">其他已配置：</span>
            {customCharDevs.map((dev) => (
              <Badge key={dev} className="shrink-0" variant="neutral">
                <span className="font-mono">{dev}</span>
              </Badge>
            ))}
          </div>
        ) : null}
      </div>

      {customMounts.length > 0 ? (
        <>
          <div className="pochita-divider" />
          <div className="flex flex-col gap-1">
            <span className="text-[11px] font-medium text-[var(--text-secondary)]">
              额外挂载预览
            </span>
            <div className="flex flex-wrap gap-1">
              {customMounts.map((mount) => (
                <Badge
                  key={`${mount.hostPath}:${mount.containerPath}`}
                  className="shrink-0"
                  variant="neutral"
                >
                  <span className="font-mono">
                    {mount.hostPath === mount.containerPath
                      ? mount.hostPath
                      : `${mount.hostPath} → ${mount.containerPath}`}
                    {mount.readOnly ? ' (ro)' : ' (rw)'}
                  </span>
                </Badge>
              ))}
            </div>
          </div>
        </>
      ) : null}
    </Card>
  );
}
