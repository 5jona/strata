import { Badge, Button, Card, EmptyState, StatusBanner, Switch } from '@gojona/ui';
import { ArrowUpCircle, Lock, Package, Trash2 } from 'lucide-react';
import type {
  InstalledToolItem,
  ToolGroupKind,
  ToolManagerKind,
  ToolsListResult,
} from '../../types/protocol.ts';

const GROUP_OPTIONS: Array<{ value: 'all' | ToolGroupKind; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'core-bridge', label: '核心桥接' },
  { value: 'mise-runtimes', label: 'mise 运行时' },
  { value: 'cli-dev', label: 'CLI 开发' },
  { value: 'debug-l3-l5', label: 'L3-L5 调试' },
  { value: 'python-uv', label: 'Python / uv' },
];

const PROTECTED_CORE_PACKAGES = new Set([
  'dropbear',
  'dropbear-bin',
  'openssh-sftp-server',
  'ca-certificates',
  'libc6',
  'bash',
  'host-exec',
  'strata-debug',
]);

function isProtectedTool(item: InstalledToolItem): boolean {
  return (
    item.manager === 'system' ||
    item.group === 'core-bridge' ||
    PROTECTED_CORE_PACKAGES.has(item.name.toLowerCase())
  );
}

function managerLabel(manager: ToolManagerKind): string {
  return manager === 'system' ? 'bridge' : manager;
}

export interface ToolsInstalledTabProps {
  listResult: ToolsListResult | null;
  groupFilter: 'all' | ToolGroupKind;
  checkOutdated: boolean;
  busy: boolean;
  onGroupFilterChange: (group: 'all' | ToolGroupKind) => void;
  onCheckOutdatedChange: (checked: boolean) => void;
  onUpgradeAll: () => void;
  onUpgradeTool: (item: InstalledToolItem) => void;
  onUninstallTool: (item: InstalledToolItem) => void;
}

/**
 * Grouped inventory of installed container tools (`mise`, `uv`, `apt`, and core bridges).
 */
export function ToolsInstalledTab({
  listResult,
  groupFilter,
  checkOutdated,
  busy,
  onGroupFilterChange,
  onCheckOutdatedChange,
  onUpgradeAll,
  onUpgradeTool,
  onUninstallTool,
}: ToolsInstalledTabProps) {
  const items = listResult?.items ?? [];
  const filtered =
    groupFilter === 'all' ? items : items.filter((item) => item.group === groupFilter);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-1">
          {GROUP_OPTIONS.map((opt) => (
            <Button
              key={opt.value}
              size="sm"
              variant={groupFilter === opt.value ? 'pill' : 'ghost'}
              onClick={() => onGroupFilterChange(opt.value)}
            >
              {opt.label}
            </Button>
          ))}
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
            <Switch checked={checkOutdated} onCheckedChange={onCheckOutdatedChange} />
            <span>检查可升级版本</span>
          </div>
          <Button size="sm" variant="secondary" disabled={busy} onClick={onUpgradeAll}>
            <ArrowUpCircle className="size-3.5" />
            全部升级
          </Button>
        </div>
      </div>

      {listResult?.outdatedWarning && (
        <StatusBanner
          tone="warning"
          title="版本检查提示"
          description={listResult.outdatedWarning}
        />
      )}

      {filtered.length === 0 ? (
        <EmptyState
          icon={<Package className="size-8" />}
          title="当前分组暂无已安装工具"
          description="请确认容器已启动，或前往「工具目录与安装」安装所需工具链。"
        />
      ) : (
        <Card className="overflow-hidden p-0">
          <div className="overflow-x-auto">
            <table className="w-full border-collapse text-left text-xs">
              <thead>
                <tr className="border-b border-[var(--border-hairline)] bg-[var(--surface-elevated)] text-[var(--text-secondary)]">
                  <th className="px-3.5 py-2.5 font-medium">工具</th>
                  <th className="px-3 py-2.5 font-medium">管理器</th>
                  <th className="px-3 py-2.5 font-medium">分组</th>
                  <th className="px-3 py-2.5 font-medium">版本</th>
                  <th className="px-3 py-2.5 font-medium">路径</th>
                  <th className="px-3 py-2.5 font-medium">状态</th>
                  <th className="px-3.5 py-2.5 text-right font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((item) => {
                  const locked = isProtectedTool(item);
                  return (
                    <tr
                      key={`${item.manager}:${item.id}:${item.version}`}
                      className="border-b border-[var(--border-hairline)] last:border-b-0 hover:bg-[var(--surface-elevated)]/45"
                    >
                      <td className="px-3.5 py-2.5 font-medium text-[var(--text-primary)]">
                        {item.name}
                      </td>
                      <td className="px-3 py-2.5">
                        <Badge variant="accent">{managerLabel(item.manager)}</Badge>
                      </td>
                      <td className="px-3 py-2.5 font-mono text-[11px] text-[var(--text-secondary)]">
                        {item.group}
                      </td>
                      <td className="px-3 py-2.5 font-mono text-[var(--text-primary)]">
                        {item.version}
                        {item.outdated && item.latestVersion && (
                          <span className="ml-1.5 text-[var(--warning)]">
                            → {item.latestVersion}
                          </span>
                        )}
                      </td>
                      <td className="max-w-[220px] truncate px-3 py-2.5 font-mono text-[11px] text-[var(--text-secondary)]">
                        {item.installPath ?? item.sourcePath ?? '—'}
                      </td>
                      <td className="px-3 py-2.5">
                        {item.broken ? (
                          <Badge variant="danger">异常</Badge>
                        ) : item.outdated ? (
                          <Badge variant="warning">可升级</Badge>
                        ) : locked ? (
                          <Badge variant="neutral">核心保护</Badge>
                        ) : (
                          <Badge variant="success">正常</Badge>
                        )}
                      </td>
                      <td className="px-3.5 py-2.5 text-right">
                        <div className="inline-flex items-center gap-1">
                          {item.manager !== 'system' && (
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={busy}
                              onClick={() => onUpgradeTool(item)}
                            >
                              <ArrowUpCircle className="size-3.5" />
                              升级
                            </Button>
                          )}
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            disabled={busy || locked}
                            title={locked ? '受保护核心包禁止卸载' : `卸载 ${item.name}`}
                            aria-label={`卸载 ${item.name}`}
                            onClick={() => onUninstallTool(item)}
                          >
                            {locked ? (
                              <Lock className="size-3.5 text-[var(--text-secondary)]" />
                            ) : (
                              <Trash2 className="size-3.5 text-[var(--danger)]" />
                            )}
                          </Button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  );
}
