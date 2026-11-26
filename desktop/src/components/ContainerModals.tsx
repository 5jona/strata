import { Button, Card, Input, SettingRow, Slider, StatusBanner, Switch } from '@gojona/ui';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { parsePortInRange } from '../lib/format.ts';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';

export type ContainerModalMode =
  | { kind: 'resize'; container: ContainerStatusSnapshot }
  | { kind: 'clone'; container: ContainerStatusSnapshot }
  | { kind: 'delete'; container: ContainerStatusSnapshot }
  | null;

export interface ContainerModalsProps {
  modal: ContainerModalMode;
  busy: boolean;
  onClose: () => void;
  onResize: (id: string, sizeMb: number) => Promise<void>;
  onClone: (sourceId: string, targetId: string, sshPort: number, hostname: string) => Promise<void>;
  onDelete: (id: string, deleteImage: boolean) => Promise<void>;
}

/**
 * Unified modal dialogs for container resize, clone, and safe deletion.
 */
export function ContainerModals({
  modal,
  busy,
  onClose,
  onResize,
  onClone,
  onDelete,
}: ContainerModalsProps) {
  const [sizeGb, setSizeGb] = useState(32);
  const [cloneId, setCloneId] = useState('');
  const [clonePort, setClonePort] = useState('2224');
  const [clonePortError, setClonePortError] = useState<string | null>(null);
  const [confirmName, setConfirmName] = useState('');
  const [deleteImage, setDeleteImage] = useState(false);

  useEffect(() => {
    if (!modal) return;
    setClonePortError(null);
    if (modal.kind === 'resize') {
      const currentGb = Math.max(
        8,
        Math.ceil(modal.container.disk.imageVirtualBytes / (1024 * 1024 * 1024)),
      );
      setSizeGb(Math.min(128, currentGb + 8));
    } else if (modal.kind === 'clone') {
      setCloneId(`${modal.container.id}-copy`);
      setClonePort(String(modal.container.sshPort + 1));
    } else if (modal.kind === 'delete') {
      setConfirmName('');
      setDeleteImage(false);
    }
  }, [modal]);

  if (!modal) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
    >
      <Card className="flex w-full max-w-md flex-col gap-4 p-5">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-base font-semibold text-[var(--text-primary)]">
            {modal.kind === 'resize' && `扩容容器镜像 · ${modal.container.id}`}
            {modal.kind === 'clone' && `克隆容器 · ${modal.container.id}`}
            {modal.kind === 'delete' && `删除容器 · ${modal.container.id}`}
          </h3>
          <Button size="icon-sm" variant="ghost" aria-label="关闭" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        {modal.kind === 'resize' && (
          <div className="flex flex-col gap-3">
            <Slider
              label="目标虚拟容量"
              min={8}
              max={128}
              step={4}
              unit=" GiB"
              value={sizeGb}
              presets={[
                { label: '16 GiB', value: 16 },
                { label: '32 GiB', value: 32 },
                { label: '64 GiB', value: 64 },
                { label: '128 GiB', value: 128 },
              ]}
              onValueChange={setSizeGb}
            />
            <p className="text-xs text-[var(--text-secondary)]">
              ext4 稀疏镜像按需占用实际空间，扩容期间容器将短暂重启并自动调整文件系统大小。
            </p>
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="secondary" onClick={onClose}>
                取消
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy}
                onClick={async () => {
                  await onResize(modal.container.id, sizeGb * 1024);
                  onClose();
                }}
              >
                确认扩容
              </Button>
            </div>
          </div>
        )}

        {modal.kind === 'clone' && (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">新容器 ID / 主机名</span>
              <Input mono value={cloneId} onChange={(e) => setCloneId(e.target.value)} />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">
                新容器 SSH 端口 (1024–65535)
              </span>
              <Input
                mono
                inputMode="numeric"
                value={clonePort}
                aria-invalid={clonePortError !== null}
                onChange={(e) => {
                  setClonePort(e.target.value);
                  if (clonePortError && parsePortInRange(e.target.value, 1024, 65535) !== null) {
                    setClonePortError(null);
                  }
                }}
              />
            </div>
            {clonePortError && (
              <p role="alert" className="text-xs text-[var(--danger)]">
                {clonePortError}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="secondary" onClick={onClose}>
                取消
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || !cloneId.trim()}
                onClick={async () => {
                  const id = cloneId.trim();
                  const parsedPort = parsePortInRange(clonePort, 1024, 65535);
                  if (parsedPort === null) {
                    setClonePortError('SSH 端口须为 1024–65535 之间的整数');
                    return;
                  }
                  await onClone(modal.container.id, id, parsedPort, id);
                  onClose();
                }}
              >
                开始克隆
              </Button>
            </div>
          </div>
        )}

        {modal.kind === 'delete' && (
          <div className="flex flex-col gap-3">
            <StatusBanner
              tone="danger"
              title="危险操作确认"
              description={`将停止容器 ${modal.container.id} 并从配置中移除。请输入容器名称以确认。`}
            />
            <Input
              mono
              value={confirmName}
              placeholder={`输入 ${modal.container.id} 确认删除`}
              onChange={(e) => setConfirmName(e.target.value)}
            />
            <SettingRow
              variant="plain"
              title="同时删除磁盘镜像文件 (.img)"
              description="勾选后将永久清空根文件系统数据，无法恢复"
              control={<Switch checked={deleteImage} onCheckedChange={setDeleteImage} />}
            />
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="secondary" onClick={onClose}>
                取消
              </Button>
              <Button
                size="sm"
                variant="danger"
                disabled={busy || confirmName.trim() !== modal.container.id}
                onClick={async () => {
                  await onDelete(modal.container.id, deleteImage);
                  onClose();
                }}
              >
                永久删除
              </Button>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}
