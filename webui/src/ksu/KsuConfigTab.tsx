import { Button, Card, Input, SettingRow, Switch } from '@gojona/ui';
import { Save } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { parseSshPort } from '../shared/strataFormat.ts';
import {
  type BindMount,
  type ContainerConfig,
  type ContainerConfigPatchParams,
  STRATA_PROTOCOL,
  type WebStatusResult,
} from '../types.ts';
import { DebugPassthroughCard } from './DebugPassthroughCard.tsx';
import { PcPairingCard } from './PcPairingCard.tsx';
import { SshKeysCard } from './SshKeysCard.tsx';

/** Props accepted by `KsuConfigTab`. */
export interface KsuConfigTabProps {
  config: ContainerConfig | null;
  sshKeys: readonly string[];
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  busy: boolean;
  onSaveConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  onAddSshKey: (publicKey: string) => Promise<boolean>;
  onRemoveSshKey: (fingerprintOrKey: string) => Promise<boolean>;
  onToggleLan: (lanEnabled: boolean) => Promise<void>;
  onReadToken: () => Promise<void>;
  onRotateToken: () => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Tab 3: container runtime settings, debug passthrough, SSH key management, and PC pairing. */
export function KsuConfigTab({
  config,
  sshKeys,
  webStatus,
  pairingToken,
  busy,
  onSaveConfig,
  onAddSshKey,
  onRemoveSshKey,
  onToggleLan,
  onReadToken,
  onRotateToken,
  onCopyText,
}: KsuConfigTabProps) {
  const hostnameId = useId();
  const sshPortId = useId();

  const [hostname, setHostname] = useState(config?.hostname ?? '');
  const [sshPort, setSshPort] = useState(
    String(config?.ssh.port ?? STRATA_PROTOCOL.DEFAULT_SSH_PORT),
  );
  const [sshPortError, setSshPortError] = useState<string | null>(null);
  const [autoStart, setAutoStart] = useState(config?.autoStart ?? true);
  const [hostBridge, setHostBridge] = useState(config?.hostBridge ?? true);
  const [noatime, setNoatime] = useState(config?.noatime ?? true);
  const [hostRuntime, setHostRuntime] = useState(config?.hostRuntime ?? false);
  const [extraCharDevs, setExtraCharDevs] = useState<string[]>(config?.extraCharDevs ?? []);
  const [extraMounts, setExtraMounts] = useState<BindMount[]>(config?.extraMounts ?? []);

  useEffect(() => {
    if (!config) return;
    setHostname(config.hostname);
    setSshPort(String(config.ssh.port));
    setSshPortError(null);
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setNoatime(config.noatime);
    setHostRuntime(config.hostRuntime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config]);

  const handleSave = async () => {
    const parsedPort = parseSshPort(sshPort);
    if (parsedPort === null) {
      setSshPortError('SSH 端口需为 1–65535 之间的有效整数');
      return;
    }
    setSshPortError(null);
    await onSaveConfig({
      hostname: hostname.trim() || undefined,
      sshPort: parsedPort,
      autoStart,
      hostBridge,
      noatime,
      hostRuntime,
      extraCharDevs,
      extraMounts,
    });
  };

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
      <div className="flex flex-col gap-3 md:col-span-6">
        <Card className="flex flex-col gap-2.5 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="truncate text-xs font-medium text-[var(--text-primary)]">
              容器基础参数
            </span>
            <Button
              className="shrink-0"
              size="sm"
              variant="primary"
              disabled={busy || !config}
              onClick={() => void handleSave()}
            >
              <Save className="size-3.5" />
              保存配置
            </Button>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="flex flex-col gap-1">
              <label htmlFor={hostnameId} className="text-[11px] text-[var(--text-secondary)]">
                主机名
              </label>
              <Input
                id={hostnameId}
                mono
                value={hostname}
                aria-label="容器主机名"
                onChange={(e) => setHostname(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1">
              <label htmlFor={sshPortId} className="text-[11px] text-[var(--text-secondary)]">
                SSH 端口
              </label>
              <Input
                id={sshPortId}
                mono
                inputMode="numeric"
                value={sshPort}
                aria-label="Dropbear SSH 端口"
                aria-invalid={sshPortError !== null}
                onChange={(e) => {
                  setSshPort(e.target.value);
                  if (sshPortError && parseSshPort(e.target.value) !== null) {
                    setSshPortError(null);
                  }
                }}
              />
            </div>
          </div>

          {sshPortError ? (
            <p role="alert" className="text-[11px] text-[var(--danger)]">
              {sshPortError}
            </p>
          ) : null}

          <div className="pochita-divider" />
          <SettingRow
            variant="plain"
            title="开机自启"
            description="stratad 启动时自动挂载并拉起 Dropbear"
            control={
              <Switch checked={autoStart} onCheckedChange={setAutoStart} aria-label="开机自启" />
            }
          />
          <div className="pochita-divider" />
          <SettingRow
            variant="plain"
            title="Android 宿主机桥接"
            description="启用 host-exec 与宿主机调试桥接能力"
            control={
              <Switch
                checked={hostBridge}
                onCheckedChange={setHostBridge}
                aria-label="Android 宿主机桥接"
              />
            }
          />
          <div className="pochita-divider" />
          <SettingRow
            variant="plain"
            title="挂载启用 noatime"
            description="减少 rootfs 元数据写入，提升 ext4 镜像 I/O 性能"
            control={
              <Switch
                checked={noatime}
                onCheckedChange={setNoatime}
                aria-label="挂载启用 noatime"
              />
            }
          />
        </Card>

        <DebugPassthroughCard
          hostRuntime={hostRuntime}
          onHostRuntimeChange={setHostRuntime}
          extraCharDevs={extraCharDevs}
          onExtraCharDevsChange={setExtraCharDevs}
          extraMounts={extraMounts}
          onExtraMountsChange={setExtraMounts}
          disabled={busy || !config}
        />
      </div>

      <div className="flex flex-col gap-3 md:col-span-6">
        <SshKeysCard
          sshKeys={sshKeys}
          busy={busy}
          onAddSshKey={onAddSshKey}
          onRemoveSshKey={onRemoveSshKey}
        />

        <PcPairingCard
          webStatus={webStatus}
          pairingToken={pairingToken}
          busy={busy}
          onToggleLan={onToggleLan}
          onReadToken={onReadToken}
          onRotateToken={onRotateToken}
          onCopyText={onCopyText}
        />
      </div>
    </div>
  );
}
