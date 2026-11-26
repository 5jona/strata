import { Badge, Button, Card } from '@gojona/ui';
import { ArrowUpCircle, Check, Lock, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import type { InstalledToolItem, ToolManagerKind, ToolUseScope } from '../../types/protocol.ts';
import {
  type GroupedToolEntry,
  isProtectedTool,
  managerLabel,
  toolGroupLabel,
  toolSourceMeta,
} from './toolHelpers.ts';

export interface InstalledToolGroupCardProps {
  groupEntry: GroupedToolEntry;
  disabled: boolean;
  onUpgradeTool: (item: InstalledToolItem) => void;
  onUninstallVersion: (item: InstalledToolItem) => void;
  onUseVersion: (
    manager: ToolManagerKind,
    name: string,
    version: string,
    scope: ToolUseScope,
  ) => void;
  onOpenVersionPicker: (
    mode: 'install' | 'use',
    manager: ToolManagerKind,
    name: string,
    ver?: string,
  ) => void;
}

/** Single grouped tool card showing active version, provenance, and multi-version actions. */
export function InstalledToolGroupCard({
  groupEntry: grp,
  disabled,
  onUpgradeTool,
  onUninstallVersion,
  onUseVersion,
  onOpenVersionPicker,
}: InstalledToolGroupCardProps) {
  const [confirmKey, setConfirmKey] = useState<string | null>(null);
  const active = grp.activeItem;
  const srcMeta = toolSourceMeta(grp.source);
  const supportsVersions = grp.manager === 'mise' || grp.manager === 'uv';
  const canUpgrade = grp.manager !== 'system' && grp.manager !== 'manual';

  return (
    <Card className="flex flex-col gap-2.5 p-3.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--text-primary)]">{grp.name}</span>
          <Badge variant="accent">{managerLabel(grp.manager)}</Badge>
          <Badge variant="neutral">{toolGroupLabel(grp.group)}</Badge>
          <Badge variant={srcMeta.variant}>{srcMeta.label}</Badge>
          <span className="font-mono text-xs text-[var(--text-primary)]">
            {active.active ? `当前: ${active.version}` : `已装: ${active.version}`}
            {active.requestedVersion && ` (约束 ${active.requestedVersion})`}
          </span>
          {active.outdated && active.latestVersion && (
            <Badge variant="warning">可升级 → {active.latestVersion}</Badge>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          {supportsVersions && (
            <Button
              size="sm"
              variant="ghost"
              disabled={disabled}
              onClick={() => onOpenVersionPicker('install', grp.manager, grp.name, active.version)}
            >
              <Plus className="size-3.5" />
              安装其他版本
            </Button>
          )}
          {canUpgrade && (
            <Button
              size="sm"
              variant="secondary"
              disabled={disabled}
              onClick={() => onUpgradeTool(active)}
            >
              <ArrowUpCircle className="size-3.5" />
              升级
            </Button>
          )}
        </div>
      </div>

      {(active.sourcePath || active.installPath) && (
        <div className="flex flex-wrap gap-x-4 gap-y-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
          {active.sourcePath && <span>配置来源：{active.sourcePath}</span>}
          {active.installPath && <span>安装路径：{active.installPath}</span>}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        {grp.versions.map((verItem) => {
          const locked = isProtectedTool(verItem);
          const verKey = `${verItem.manager}:${verItem.name}@${verItem.version}`;
          const confirming = confirmKey === verKey;
          return (
            <div
              key={verKey}
              className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs ${
                verItem.active
                  ? 'border-[var(--accent)]/50 bg-[var(--accent)]/10'
                  : 'border-[var(--border-hairline)] bg-[var(--surface-elevated)]'
              }`}
            >
              <span className="font-mono font-medium text-[var(--text-primary)]">
                {verItem.version}
              </span>
              {verItem.active && <Badge variant="success">激活中</Badge>}
              {verItem.broken && <Badge variant="danger">异常</Badge>}
              {grp.manager === 'mise' && !verItem.active && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={disabled}
                  onClick={() => onUseVersion('mise', grp.name, verItem.version, 'global')}
                >
                  <Check className="size-3" />
                  设为全局
                </Button>
              )}
              {grp.manager === 'mise' && (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={disabled}
                  onClick={() => onOpenVersionPicker('use', 'mise', grp.name, verItem.version)}
                >
                  切换作用域
                </Button>
              )}
              {locked ? (
                <span
                  title="核心组件受保护，禁止卸载"
                  className="px-1 text-[var(--text-secondary)]"
                >
                  <Lock className="size-3" />
                </span>
              ) : confirming ? (
                <div className="flex items-center gap-1">
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={disabled}
                    onClick={() => {
                      setConfirmKey(null);
                      onUninstallVersion(verItem);
                    }}
                  >
                    确认卸载
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setConfirmKey(null)}>
                    取消
                  </Button>
                </div>
              ) : (
                <Button
                  size="icon-sm"
                  variant="ghost"
                  disabled={disabled}
                  title={`卸载 ${verItem.name}@${verItem.version}`}
                  aria-label={`卸载 ${verItem.name} ${verItem.version}`}
                  onClick={() => setConfirmKey(verKey)}
                >
                  <Trash2 className="size-3 text-[var(--danger)]" />
                </Button>
              )}
            </div>
          );
        })}
      </div>
    </Card>
  );
}
