import { Badge, Button, Input, StatusBanner, Switch } from '@gojona/ui';
import { Plus, Trash2, X } from 'lucide-react';
import { useState } from 'react';
import {
  COMMON_DEBUG_CHAR_DEVS,
  getCustomCharDevs,
  getCustomMounts,
  toggleCharDev,
} from '../lib/passthrough.ts';
import type { BindMount } from '../types/protocol.ts';

export interface PassthroughEditorProps {
  extraCharDevs: string[];
  extraMounts: BindMount[];
  onCharDevsChange: (next: string[]) => void;
  onMountsChange: (next: BindMount[]) => void;
}

/**
 * Character device (`extraCharDevs`) and custom `BindMount` list editor.
 */
export function PassthroughEditor({
  extraCharDevs,
  extraMounts,
  onCharDevsChange,
  onMountsChange,
}: PassthroughEditorProps) {
  const [newCharDev, setNewCharDev] = useState('');
  const [newHostPath, setNewHostPath] = useState('');
  const [newContainerPath, setNewContainerPath] = useState('');
  const [newReadOnly, setNewReadOnly] = useState(true);
  const [mountError, setMountError] = useState<string | null>(null);

  const customMounts = getCustomMounts(extraMounts);
  const customCharDevs = getCustomCharDevs(extraCharDevs);

  const handleAddMount = () => {
    const hostPath = newHostPath.trim();
    const containerPath = (newContainerPath.trim() || hostPath).trim();
    if (!hostPath.startsWith('/') || !containerPath.startsWith('/')) {
      setMountError('宿主机路径与容器路径均须以 `/` 开头');
      return;
    }
    if (customMounts.some((m) => m.containerPath === containerPath)) {
      setMountError(`容器路径 ${containerPath} 已存在挂载配置`);
      return;
    }
    setMountError(null);
    onMountsChange([...extraMounts, { hostPath, containerPath, readOnly: newReadOnly }]);
    setNewHostPath('');
    setNewContainerPath('');
  };

  const handleAddCharDev = () => {
    const dev = newCharDev.trim().replace(/^\/dev\//, '');
    if (!dev) return;
    onCharDevsChange(toggleCharDev(extraCharDevs, dev, true));
    setNewCharDev('');
  };

  return (
    <>
      <div className="flex flex-col gap-1.5 border-t border-[var(--border-hairline)] pt-2.5">
        <span className="text-xs font-medium text-[var(--text-primary)]">
          字符设备透传（extraCharDevs）
        </span>
        <div className="flex flex-wrap items-center gap-1.5">
          {COMMON_DEBUG_CHAR_DEVS.map((opt) => {
            const active = extraCharDevs.includes(opt.name);
            return (
              <Button
                key={opt.name}
                size="sm"
                variant={active ? 'pill' : 'secondary'}
                aria-pressed={active}
                onClick={() => onCharDevsChange(toggleCharDev(extraCharDevs, opt.name, !active))}
              >
                <span className="font-mono">{opt.label}</span>
                <span className="text-[10px] opacity-80">{opt.description}</span>
              </Button>
            );
          })}
          {customCharDevs.map((dev) => (
            <Button
              key={dev}
              size="sm"
              variant="pill"
              onClick={() => onCharDevsChange(toggleCharDev(extraCharDevs, dev, false))}
            >
              <span className="font-mono">{dev}</span>
              <X className="size-3" />
            </Button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <Input
            mono
            value={newCharDev}
            placeholder="自定义 /dev/* 相对路径，例如：kvm 或 dri/renderD128"
            onChange={(e) => setNewCharDev(e.target.value)}
          />
          <Button
            size="sm"
            variant="secondary"
            disabled={!newCharDev.trim()}
            onClick={handleAddCharDev}
          >
            <Plus className="size-3.5" />
            添加
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-2 border-t border-[var(--border-hairline)] pt-2.5">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-[var(--text-primary)]">
            自定义目录挂载（BindMount）
          </span>
          <Badge variant="neutral">{customMounts.length} 项</Badge>
        </div>
        {mountError && <StatusBanner tone="warning" title={mountError} />}
        {customMounts.length > 0 && (
          <div className="flex flex-col gap-1.5">
            {customMounts.map((mount) => (
              <div
                key={`${mount.hostPath}:${mount.containerPath}`}
                className="flex items-center justify-between gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5 text-xs"
              >
                <span className="min-w-0 flex-1 truncate font-mono text-[var(--text-primary)]">
                  {mount.hostPath} → {mount.containerPath}
                </span>
                <Badge variant={mount.readOnly ? 'neutral' : 'warning'}>
                  {mount.readOnly ? 'ro' : 'rw'}
                </Badge>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={`移除挂载 ${mount.containerPath}`}
                  onClick={() =>
                    onMountsChange(
                      extraMounts.filter(
                        (m) =>
                          !(
                            m.hostPath === mount.hostPath && m.containerPath === mount.containerPath
                          ),
                      ),
                    )
                  }
                >
                  <Trash2 className="size-3.5 text-[var(--danger)]" />
                </Button>
              </div>
            ))}
          </div>
        )}
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <Input
            mono
            value={newHostPath}
            placeholder="宿主机路径，如 /sdcard/Download"
            onChange={(e) => setNewHostPath(e.target.value)}
          />
          <Input
            mono
            value={newContainerPath}
            placeholder="容器路径，如 /mnt/download"
            onChange={(e) => setNewContainerPath(e.target.value)}
          />
        </div>
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
            <Switch checked={newReadOnly} onCheckedChange={setNewReadOnly} />
            <span>{newReadOnly ? '只读挂载 (ro)' : '读写挂载 (rw)'}</span>
          </div>
          <Button size="sm" variant="secondary" onClick={handleAddMount}>
            <Plus className="size-3.5" />
            添加挂载
          </Button>
        </div>
      </div>
    </>
  );
}
