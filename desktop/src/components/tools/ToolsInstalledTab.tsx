import { Button, EmptyState, Input, SegmentedControl, StatusBanner, Switch } from '@gojona/ui';
import { ArrowUpCircle, Package, Search } from 'lucide-react';
import { useState } from 'react';
import type {
  InstalledToolItem,
  ToolGroupKind,
  ToolInstallSource,
  ToolManagerKind,
  ToolsListResult,
  ToolUseScope,
} from '../../types/protocol.ts';
import { InstalledToolGroupCard } from './InstalledToolGroupCard.tsx';
import { groupInstalledTools, TOOL_GROUP_LABELS, TOOL_GROUP_ORDER } from './toolHelpers.ts';

export interface ToolsInstalledTabProps {
  listResult: ToolsListResult | null;
  groupFilter: 'all' | ToolGroupKind;
  checkOutdated: boolean;
  containerRunning: boolean;
  busy: boolean;
  onGroupFilterChange: (group: 'all' | ToolGroupKind) => void;
  onCheckOutdatedChange: (checked: boolean) => void;
  onUpgradeAll: () => void;
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

/**
 * Grouped inventory of installed & auto-discovered container tools with multi-version switching.
 */
export function ToolsInstalledTab({
  listResult,
  groupFilter,
  checkOutdated,
  containerRunning,
  busy,
  onGroupFilterChange,
  onCheckOutdatedChange,
  onUpgradeAll,
  onUpgradeTool,
  onUninstallVersion,
  onUseVersion,
  onOpenVersionPicker,
}: ToolsInstalledTabProps) {
  const [sourceFilter, setSourceFilter] = useState<'all' | ToolInstallSource>('all');
  const [query, setQuery] = useState('');
  const [onlyOutdated, setOnlyOutdated] = useState(false);

  const rawItems = listResult?.items ?? [];
  const q = query.trim().toLowerCase();
  const filteredItems = rawItems.filter((item) => {
    if (groupFilter !== 'all' && item.group !== groupFilter) return false;
    if (sourceFilter !== 'all' && (item.source ?? 'system') !== sourceFilter) return false;
    if (onlyOutdated && !item.outdated) return false;
    if (
      q &&
      !item.name.toLowerCase().includes(q) &&
      !(item.installPath ?? '').toLowerCase().includes(q)
    ) {
      return false;
    }
    return true;
  });

  const groups = groupInstalledTools(filteredItems);
  const disabled = busy || !containerRunning;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-1">
          {TOOL_GROUP_ORDER.map((key) => (
            <Button
              key={key}
              size="sm"
              variant={groupFilter === key ? 'pill' : 'ghost'}
              onClick={() => onGroupFilterChange(key)}
            >
              {TOOL_GROUP_LABELS[key]}
            </Button>
          ))}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
            <Switch checked={checkOutdated} onCheckedChange={onCheckOutdatedChange} />
            <span>检查更新</span>
          </div>
          <Button size="sm" variant="secondary" disabled={disabled} onClick={onUpgradeAll}>
            <ArrowUpCircle className="size-3.5" />
            全部升级
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <SegmentedControl
            size="sm"
            value={sourceFilter}
            onValueChange={setSourceFilter}
            options={[
              { value: 'all', label: '全部来源' },
              { value: 'user-installed', label: '用户自装' },
              { value: 'strata-managed', label: 'Strata 托管' },
              { value: 'system', label: '系统自带' },
            ]}
          />
          <div className="flex items-center gap-1.5 text-xs text-[var(--text-secondary)]">
            <Switch checked={onlyOutdated} onCheckedChange={setOnlyOutdated} />
            <span>仅看可升级</span>
          </div>
        </div>
        <div className="relative w-60">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
          <Input
            className="pl-8"
            value={query}
            placeholder="过滤工具名称或路径…"
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </div>

      {listResult?.outdatedWarning && (
        <StatusBanner
          tone="warning"
          title="版本检查提示"
          description={listResult.outdatedWarning}
        />
      )}

      {groups.length === 0 ? (
        <EmptyState
          icon={<Package className="size-8" />}
          title={containerRunning ? '当前筛选条件暂无已安装工具' : '容器未启动，暂无法读取工具列表'}
          description="可切换筛选条件，或前往「安装工具与终端预设」添加所需工具。"
        />
      ) : (
        <div className="flex flex-col gap-2">
          {groups.map((grp) => (
            <InstalledToolGroupCard
              key={grp.key}
              groupEntry={grp}
              disabled={disabled}
              onUpgradeTool={onUpgradeTool}
              onUninstallVersion={onUninstallVersion}
              onUseVersion={onUseVersion}
              onOpenVersionPicker={onOpenVersionPicker}
            />
          ))}
        </div>
      )}
    </div>
  );
}
