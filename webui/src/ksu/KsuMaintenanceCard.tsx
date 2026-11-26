import { Button, Card, StatusBanner, Switch } from '@gojona/ui';
import { Archive, RefreshCw, ShieldCheck, Sparkles, Star, Trash2, Wifi } from 'lucide-react';
import { useId, useState } from 'react';
import type { ContainerStatusSnapshot } from '../types.ts';
import type { KsuMaintenanceOp } from './useKsuActions.ts';

export interface KsuMaintenanceCardProps {
  container: ContainerStatusSnapshot;
  busyKey: string | null;
  onMaintenance: (op: KsuMaintenanceOp) => Promise<void>;
  onSetDefault: (containerId: string) => Promise<void>;
  onDeleteContainer: (containerId: string, deleteImage: boolean) => Promise<void>;
}

/** Disk/network maintenance and container default/delete lifecycle actions. */
export function KsuMaintenanceCard({
  container,
  busyKey,
  onMaintenance,
  onSetDefault,
  onDeleteContainer,
}: KsuMaintenanceCardProps) {
  const deleteImgId = useId();
  const [confirmFsck, setConfirmFsck] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteImage, setDeleteImage] = useState(true);

  const busy = busyKey !== null;
  const imageMissing = !container.disk.imageExists;

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs font-medium text-[var(--text-primary)]">磁盘维护与容器管理</span>
        <span className="text-[11px] text-[var(--text-secondary)]">
          按顺序安全执行，支持无 CLI 管理初始容器
        </span>
      </div>

      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || !container.mounted}
          onClick={() => void onMaintenance('dns')}
        >
          <Wifi className="size-3.5" />
          同步 DNS
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || imageMissing}
          onClick={() => void onMaintenance('compact')}
        >
          <Sparkles className="size-3.5" />
          回收空闲空间
        </Button>
        <Button
          size="sm"
          variant={confirmFsck ? 'danger' : 'secondary'}
          disabled={busy || imageMissing}
          onClick={() => {
            setConfirmDelete(false);
            setConfirmFsck((prev) => !prev);
          }}
        >
          <ShieldCheck className="size-3.5" />
          停机磁盘检查
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || imageMissing}
          onClick={() => void onMaintenance('backup')}
        >
          {busyKey === 'maint:backup' ? (
            <RefreshCw className="size-3.5 animate-spin" />
          ) : (
            <Archive className="size-3.5" />
          )}
          本地镜像备份
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || container.isDefault}
          onClick={() => void onSetDefault(container.id)}
        >
          <Star className="size-3.5" />
          {container.isDefault ? '已是默认容器' : '设为默认容器'}
        </Button>
        <Button
          size="sm"
          variant={confirmDelete ? 'danger' : 'secondary'}
          disabled={busy}
          onClick={() => {
            setConfirmFsck(false);
            setConfirmDelete((prev) => !prev);
          }}
        >
          <Trash2 className="size-3.5" />
          删除容器
        </Button>
      </div>

      <p className="text-[11px] text-[var(--text-secondary)]">
        提示：「回收空闲空间」将压缩镜像未用块；「停机磁盘检查」会先安全停止容器再校验修复。
      </p>

      {confirmFsck ? (
        <StatusBanner
          tone="warning"
          title="确认执行停机磁盘检查 (e2fsck)？"
          description="该操作会先安全停止当前容器并卸载镜像，再检查并修复 ext4 文件系统。"
          action={
            <div className="flex items-center gap-1.5">
              <Button
                size="sm"
                variant="danger"
                disabled={busy}
                onClick={() => {
                  setConfirmFsck(false);
                  void onMaintenance('fsck');
                }}
              >
                确认检查
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirmFsck(false)}>
                取消
              </Button>
            </div>
          }
        />
      ) : null}

      {confirmDelete ? (
        <StatusBanner
          tone="danger"
          title={`确认删除容器「${container.id}」？`}
          description="将先安全停止并卸载该容器，随后从配置中移除（已启用强制删除，支持删除默认或最后一个容器）。"
          action={
            <div className="flex flex-wrap items-center gap-2">
              <label
                htmlFor={deleteImgId}
                className="flex cursor-pointer items-center gap-1.5 text-xs text-[var(--text-primary)]"
              >
                <Switch
                  id={deleteImgId}
                  checked={deleteImage}
                  disabled={busy}
                  aria-label="同时删除磁盘镜像"
                  onCheckedChange={setDeleteImage}
                />
                同时删除磁盘镜像
              </label>
              <Button
                size="sm"
                variant="danger"
                disabled={busy}
                onClick={() => {
                  setConfirmDelete(false);
                  void onDeleteContainer(container.id, deleteImage);
                }}
              >
                确认删除
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>
                取消
              </Button>
            </div>
          }
        />
      ) : null}
    </Card>
  );
}
