import { Badge, Button, Card, Input, SettingRow, Switch } from '@gojona/ui';
import { X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { parsePortInRange } from '../lib/format.ts';
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
  const [sshEnabled, setSshEnabled] = useState(false);
  const [sshPort, setSshPort] = useState('2222');
  const [sshPortError, setSshPortError] = useState<string | null>(null);
  const [autoStart, setAutoStart] = useState(false);
  const [hostBridge, setHostBridge] = useState(true);
  const [hostRuntime, setHostRuntime] = useState(false);
  const [noatime, setNoatime] = useState(true);
  const [extraCharDevs, setExtraCharDevs] = useState<string[]>([]);
  const [extraMounts, setExtraMounts] = useState<BindMount[]>([]);

  useEffect(() => {
    if (!open || !config) return;
    setHostname(config.hostname);
    setSshEnabled(config.ssh.enabled);
    setSshPort(String(config.ssh.port));
    setSshPortError(null);
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setHostRuntime(config.hostRuntime);
    setNoatime(config.noatime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config, open]);

  if (!open || !container || !config) return null;

  const l5Enabled = hasL5DiagMounts(extraMounts);

  const handleSave = async () => {
    let parsedPort: number | undefined;
    if (sshEnabled) {
      const port = parsePortInRange(sshPort, 1024, 65535);
      if (port === null) {
        setSshPortError('SSH 端口须为 1024–65535 之间的整数');
        return;
      }
      parsedPort = port;
    }
    setSshPortError(null);
    await onSave({
      container: container.id,
      hostname: hostname.trim() || undefined,
      sshEnabled,
      ...(parsedPort !== undefined ? { sshPort: parsedPort } : {}),
      autoStart,
      hostBridge,
      hostRuntime,
      noatime,
      extraCharDevs,
      extraMounts,
    });
    onClose();
  };

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
              容器配置 · {container.id}
            </h3>
            <Badge variant="neutral">部分透传需重启容器生效</Badge>
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
              <span className="text-xs text-[var(--text-secondary)]">SSH 端口 (1024–65535)</span>
              <Input
                mono
                inputMode="numeric"
                disabled={!sshEnabled}
                value={sshPort}
                aria-invalid={sshPortError !== null}
                onChange={(e) => {
                  setSshPort(e.target.value);
                  if (sshPortError && parsePortInRange(e.target.value, 1024, 65535) !== null) {
                    setSshPortError(null);
                  }
                }}
              />
            </div>
          </div>

          {sshPortError && (
            <p role="alert" className="text-xs text-[var(--danger)]">
              {sshPortError}
            </p>
          )}

          <SettingRow
            variant="plain"
            title="启用 Dropbear SSH 服务"
            description="仅公钥免密登录（纯容器未装 dropbear-bin 时请保持关闭）"
            control={
              <Switch
                checked={sshEnabled}
                onCheckedChange={(next) => {
                  setSshEnabled(next);
                  setSshPortError(null);
                  if (next && parsePortInRange(sshPort, 1024, 65535) === null) {
                    setSshPort('2222');
                  }
                }}
              />
            }
          />

          <SettingRow
            variant="plain"
            title="开机自启"
            description="后台服务启动时自动挂载并拉起该容器及其自启服务"
            control={<Switch checked={autoStart} onCheckedChange={setAutoStart} />}
          />
          <SettingRow
            variant="plain"
            title="Android 宿主机桥接 (hostBridge)"
            description="允许容器内通过 host-exec 调用 Android 宿主机调试能力"
            control={<Switch checked={hostBridge} onCheckedChange={setHostBridge} />}
          />
          <SettingRow
            variant="plain"
            title="宿主机环境直通 (高权限)"
            description="直通宿主机 /proc、/sys、/dev 与完整权限集，保存后需重启容器生效"
            control={<Switch checked={hostRuntime} onCheckedChange={setHostRuntime} />}
          />
          <SettingRow
            variant="plain"
            title="减少闪存写入 (noatime)"
            description="挂载镜像时禁用访问时间更新，提升读写性能并延长闪存寿命"
            control={<Switch checked={noatime} onCheckedChange={setNoatime} />}
          />
          <SettingRow
            variant="plain"
            title="内核深度诊断只读挂载"
            description="只读挂载 pstore、tracing 与 bpf 内核调试接口（/sys/fs/pstore、/sys/kernel/tracing、/sys/fs/bpf）"
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
          <Button size="sm" variant="primary" disabled={busy} onClick={() => void handleSave()}>
            保存配置
          </Button>
        </div>
      </Card>
    </div>
  );
}
