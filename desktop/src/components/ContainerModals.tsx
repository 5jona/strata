import { Button, Card, Input, SettingRow, Slider, StatusBanner, Switch } from '@gojona/ui';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';
import type {
  ContainerConfig,
  ContainerConfigPatchParams,
  ContainerStatusSnapshot,
} from '../types/protocol.ts';

export type ContainerModalMode =
  | { kind: 'resize'; container: ContainerStatusSnapshot }
  | { kind: 'clone'; container: ContainerStatusSnapshot }
  | { kind: 'config'; container: ContainerStatusSnapshot; config: ContainerConfig }
  | { kind: 'delete'; container: ContainerStatusSnapshot }
  | null;

export interface ContainerModalsProps {
  modal: ContainerModalMode;
  busy: boolean;
  onClose: () => void;
  onResize: (id: string, sizeMb: number) => Promise<void>;
  onClone: (sourceId: string, targetId: string, sshPort: number, hostname: string) => Promise<void>;
  onSaveConfig: (patch: ContainerConfigPatchParams) => Promise<void>;
  onDelete: (id: string, deleteImage: boolean) => Promise<void>;
}

/**
 * Unified modal dialogs for container resize, clone, advanced config, and safe deletion.
 */
export function ContainerModals({
  modal,
  busy,
  onClose,
  onResize,
  onClone,
  onSaveConfig,
  onDelete,
}: ContainerModalsProps) {
  const [sizeGb, setSizeGb] = useState(32);
  const [cloneId, setCloneId] = useState('');
  const [clonePort, setClonePort] = useState('2224');
  const [hostname, setHostname] = useState('');
  const [sshPort, setSshPort] = useState('2222');
  const [autoStart, setAutoStart] = useState(false);
  const [hostBridge, setHostBridge] = useState(true);
  const [hostRuntime, setHostRuntime] = useState(false);
  const [noatime, setNoatime] = useState(true);
  const [confirmName, setConfirmName] = useState('');
  const [deleteImage, setDeleteImage] = useState(false);

  useEffect(() => {
    if (!modal) return;
    if (modal.kind === 'resize') {
      const currentGb = Math.max(
        8,
        Math.ceil(modal.container.disk.imageVirtualBytes / (1024 * 1024 * 1024)),
      );
      setSizeGb(Math.min(128, currentGb + 8));
    } else if (modal.kind === 'clone') {
      setCloneId(`${modal.container.id}-copy`);
      setClonePort(String(modal.container.sshPort + 1));
    } else if (modal.kind === 'config') {
      setHostname(modal.config.hostname);
      setSshPort(String(modal.config.ssh.port));
      setAutoStart(modal.config.autoStart);
      setHostBridge(modal.config.hostBridge);
      setHostRuntime(modal.config.hostRuntime);
      setNoatime(modal.config.noatime);
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
            {modal.kind === 'config' && `高级配置 · ${modal.container.id}`}
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
              ext4 稀疏镜像按需占用实际空间，扩容时将自动安全卸载并执行 e2fsck + resize2fs。
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
              <span className="text-xs text-[var(--text-secondary)]">新容器 ID</span>
              <Input mono value={cloneId} onChange={(e) => setCloneId(e.target.value)} />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">新容器 SSH 端口</span>
              <Input mono value={clonePort} onChange={(e) => setClonePort(e.target.value)} />
            </div>
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="secondary" onClick={onClose}>
                取消
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy || !cloneId.trim()}
                onClick={async () => {
                  await onClone(
                    modal.container.id,
                    cloneId.trim(),
                    Number(clonePort) || 2224,
                    cloneId.trim(),
                  );
                  onClose();
                }}
              >
                开始克隆
              </Button>
            </div>
          </div>
        )}

        {modal.kind === 'config' && (
          <div className="flex flex-col gap-2.5">
            <div className="grid grid-cols-2 gap-2.5">
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">主机名</span>
                <Input mono value={hostname} onChange={(e) => setHostname(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">Dropbear SSH 端口</span>
                <Input mono value={sshPort} onChange={(e) => setSshPort(e.target.value)} />
              </div>
            </div>
            <SettingRow
              variant="plain"
              title="开机自动启动"
              description="设备启动完成后自动拉起该容器及其自启服务"
              control={<Switch checked={autoStart} onCheckedChange={setAutoStart} />}
            />
            <SettingRow
              variant="plain"
              title="宿主机桥接 (hostBridge)"
              description="允许容器通过受控通道调用宿主机调试工具"
              control={<Switch checked={hostBridge} onCheckedChange={setHostBridge} />}
            />
            <SettingRow
              variant="plain"
              title="宿主机完整运行时 (hostRuntime)"
              description="直通宿主机 /proc、/sys、/dev 与完整权限集（需重启生效）"
              control={<Switch checked={hostRuntime} onCheckedChange={setHostRuntime} />}
            />
            <SettingRow
              variant="plain"
              title="noatime 挂载"
              description="减少闪存写入放大并提升 I/O 吞吐"
              control={<Switch checked={noatime} onCheckedChange={setNoatime} />}
            />
            <div className="flex justify-end gap-2 pt-1">
              <Button size="sm" variant="secondary" onClick={onClose}>
                取消
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={busy}
                onClick={async () => {
                  await onSaveConfig({
                    container: modal.container.id,
                    hostname: hostname.trim(),
                    sshPort: Number(sshPort) || 2222,
                    autoStart,
                    hostBridge,
                    hostRuntime,
                    noatime,
                  });
                  onClose();
                }}
              >
                保存配置
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
