import { Button, EmptyState, Input } from '@gojona/ui';
import { Box, Plus, RefreshCw, Search } from 'lucide-react';

export interface ContainersHeaderProps {
  deviceLabel: string;
  loading: boolean;
  showSearch: boolean;
  query: string;
  onQueryChange: (value: string) => void;
  onRefresh: () => void;
  onNavigateInstall: () => void;
}

/** Header toolbar and search input for `ContainersView`. */
export function ContainersHeader({
  deviceLabel,
  loading,
  showSearch,
  query,
  onQueryChange,
  onRefresh,
  onNavigateInstall,
}: ContainersHeaderProps) {
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">容器 · {deviceLabel}</h1>
          <p className="text-xs text-[var(--text-secondary)]">
            一键启停、打开终端或 VS Code 远程开发；展开卡片可执行空间回收、磁盘修复与透传设置
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="secondary" disabled={loading} onClick={onRefresh}>
            <RefreshCw className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
            刷新
          </Button>
          <Button size="sm" variant="primary" onClick={onNavigateInstall}>
            <Plus className="size-3.5" />
            新建 / 导入容器
          </Button>
        </div>
      </div>

      {showSearch && (
        <div className="relative max-w-sm">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
          <Input
            className="pl-8"
            value={query}
            placeholder="搜索容器名称或主机名…"
            aria-label="搜索容器"
            onChange={(e) => onQueryChange(e.target.value)}
          />
        </div>
      )}
    </>
  );
}

export interface ContainersEmptyStateProps {
  hasAnyContainers: boolean;
  onNavigateInstall: () => void;
  onClearQuery: () => void;
}

/** Empty / zero-match state card for `ContainersView`. */
export function ContainersEmptyState({
  hasAnyContainers,
  onNavigateInstall,
  onClearQuery,
}: ContainersEmptyStateProps) {
  if (!hasAnyContainers) {
    return (
      <EmptyState
        icon={<Box className="size-10" />}
        title="开始创建你的第一个 Linux 容器"
        description="从清华 TUNA 或 LinuxContainers 官方镜像站一键拉取 Ubuntu、Debian、Arch 等 ARM64 发行版，或拖拽导入本地镜像包。"
        action={
          <Button size="sm" variant="primary" onClick={onNavigateInstall}>
            <Plus className="size-3.5" />
            新建 / 导入容器
          </Button>
        }
      />
    );
  }
  return (
    <EmptyState
      icon={<Box className="size-8" />}
      title="没有匹配的容器"
      description="请尝试调整搜索关键词，或清空搜索框查看全部容器。"
      action={
        <Button size="sm" variant="secondary" onClick={onClearQuery}>
          清空搜索
        </Button>
      }
    />
  );
}
