import { Button, Card, EmptyState, StatusBanner } from '@gojona/ui';
import { Layers } from 'lucide-react';

/** Props accepted by `StrataPanelPermissionBanner`. */
export interface StrataPanelPermissionBannerProps {
  notice: string | null;
  onDismiss: () => void;
}

/** Dismissible guidance banner shown when SideVeil blocks clipboard or HUD permissions. */
export function StrataPanelPermissionBanner({
  notice,
  onDismiss,
}: StrataPanelPermissionBannerProps) {
  if (!notice) return null;
  return (
    <StatusBanner
      tone="warning"
      title="权限尚未开启"
      description={notice}
      action={
        <Button size="sm" variant="ghost" onClick={onDismiss}>
          知道了
        </Button>
      }
    />
  );
}

/** Props accepted by `StrataPanelFallbackCard`. */
export interface StrataPanelFallbackCardProps {
  backendOnline: boolean;
  lastError: string | null;
  loading: boolean;
  onRetry: () => void;
}

/** Renders offline, connection error, loading, or zero-container fallback states in SideVeil. */
export function StrataPanelFallbackCard({
  backendOnline,
  lastError,
  loading,
  onRetry,
}: StrataPanelFallbackCardProps) {
  if (!backendOnline) {
    return (
      <StatusBanner
        tone="warning"
        title="Strata 服务暂未运行"
        description="请前往 KernelSU 打开 Strata 管理页启动后台服务。"
      />
    );
  }
  if (lastError) {
    return (
      <StatusBanner
        tone="danger"
        title="无法连接 Strata 服务"
        description={lastError}
        action={
          <Button size="sm" variant="secondary" onClick={onRetry}>
            重试
          </Button>
        }
      />
    );
  }
  if (loading) {
    return (
      <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
        正在读取 Strata 容器状态…
      </Card>
    );
  }
  return (
    <EmptyState
      icon={<Layers className="size-7" />}
      title="尚未创建容器"
      description="请前往 KernelSU 管理页开启 PC 配对，或连接电脑端 Strata Desktop 创建第一个容器。"
      action={
        <Button size="sm" variant="secondary" onClick={onRetry}>
          刷新状态
        </Button>
      }
    />
  );
}
