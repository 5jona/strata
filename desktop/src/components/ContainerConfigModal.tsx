import { Badge, Button, Card, Input, SettingRow, Switch } from '@gojona/ui';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { hasL5DiagMounts, toggleL5DiagMounts } from '../lib/passthrough.ts';
import type {
  BindMount,
  ContainerConfig,
  ContainerConfigPatchParams,
  ContainerStatusSnapshot,
} from '../types/protocol.ts';
import { PassthroughEditor } from './PassthroughEditor.tsx';

export interface ContainerConfigModalProps {
  open: boolean;
  container: ContainerStatusSnapshot | null;
  config: ContainerConfig | null;
  busy: boolean;
  onClose: () => void;
  onSave: (patch: ContainerConfigPatchParams) => Promise<void>;
}

/**
 * Modal editor for container networking, runtime passthrough, L5 diagnostic mounts,
 * character devices, and custom BindMount entries.
 */
export function ContainerConfigModal({
  open,
  container,
  config,
  busy,
  onClose,
  onSave,
}: ContainerConfigModalProps) {
  const [hostname, setHostname] = useState('');
  const [sshPort, setSshPort] = useState('2222');
  const [autoStart, setAutoStart] = useState(false);
  const [hostBridge, setHostBridge] = useState(true);
  const [hostRuntime, setHostRuntime] = useState(false);
  const [noatime, setNoatime] = useState(true);
  const [extraCharDevs, setExtraCharDevs] = useState<string[]>([]);
  const [extraMounts, setExtraMounts] = useState<BindMount[]>([]);

  useEffect(() => {
    if (!open || !config) return;
    setHostname(config.hostname);
    setSshPort(String(config.ssh.port));
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setHostRuntime(config.hostRuntime);
    setNoatime(config.noatime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config, open]);

  if (!open || !container || !config) return null;

  const l5Enabled = hasL5DiagMounts(extraMounts);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
    >
      <Card className="flex max-h-[88vh] w-full max-w-xl flex-col gap-3 overflow-hidden p-5">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              高级配置 · {container.id}
            </h3>
            <Badge variant="neutral">部分透传需重启生效</Badge>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="关闭" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1 strata-scroll">
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
            description="直通宿主机 /proc、/sys、/dev 与完整权限集"
            control={<Switch checked={hostRuntime} onCheckedChange={setHostRuntime} />}
          />
          <SettingRow
            variant="plain"
            title="noatime 挂载"
            description="减少闪存写入放大并提升 I/O 吞吐"
            control={<Switch checked={noatime} onCheckedChange={setNoatime} />}
          />
          <SettingRow
            variant="plain"
            title="L5 内核诊断只读挂载"
            description="只读挂载 /sys/fs/pstore、/sys/kernel/tracing 与 /sys/fs/bpf"
            control={
              <Switch
                checked={l5Enabled}
                onCheckedChange={(checked) =>
                  setExtraMounts((prev) => toggleL5DiagMounts(prev, checked))
                }
              />
            }
          />

          <PassthroughEditor
            extraCharDevs={extraCharDevs}
            extraMounts={extraMounts}
            onCharDevsChange={setExtraCharDevs}
            onMountsChange={setExtraMounts}
          />
        </div>

        <div className="flex justify-end gap-2 border-t border-[var(--border-hairline)] pt-3">
          <Button size="sm" variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={busy}
            onClick={async () => {
              await onSave({
                container: container.id,
                hostname: hostname.trim(),
                sshPort: Number(sshPort) || 2222,
                autoStart,
                hostBridge,
                hostRuntime,
                noatime,
                extraCharDevs,
                extraMounts,
              });
              onClose();
            }}
          >
            保存配置
          </Button>
        </div>
      </Card>
    </div>
  );
}
