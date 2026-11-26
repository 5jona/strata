import { Button, Card, Input, SettingRow, Switch } from '@gojona/ui';
import { RotateCcw, Save } from 'lucide-react';
import { useEffect, useId, useMemo, useState } from 'react';
import { parseSshPort } from '../shared/strataFormat.ts';
import {
  type BindMount,
  type ContainerConfig,
  type ContainerConfigPatchParams,
  STRATA_PROTOCOL,
} from '../types.ts';
import { DebugPassthroughCard } from './DebugPassthroughCard.tsx';
import { SshKeysCard } from './SshKeysCard.tsx';

/** Props accepted by `KsuConfigTab`. */
export interface KsuConfigTabProps {
  config: ContainerConfig | null;
  sshKeys: readonly string[];
  busy: boolean;
  onSaveConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  onAddSshKey: (publicKey: string) => Promise<boolean>;
  onRemoveSshKey: (fingerprintOrKey: string) => Promise<boolean>;
}

/** Tab 3: container runtime settings, debug passthrough, and SSH key management. */
export function KsuConfigTab({
  config,
  sshKeys,
  busy,
  onSaveConfig,
  onAddSshKey,
  onRemoveSshKey,
}: KsuConfigTabProps) {
  const hostnameId = useId();
  const sshPortId = useId();

  const [hostname, setHostname] = useState(config?.hostname ?? '');
  const [sshEnabled, setSshEnabled] = useState(config?.ssh.enabled ?? false);
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
    setSshEnabled(config.ssh.enabled);
    setSshPort(String(config.ssh.port));
    setSshPortError(null);
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setNoatime(config.noatime);
    setHostRuntime(config.hostRuntime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config]);

  const isDirty = useMemo(() => {
    if (!config) return false;
    return (
      hostname.trim() !== config.hostname ||
      sshEnabled !== config.ssh.enabled ||
      (sshEnabled && sshPort.trim() !== String(config.ssh.port)) ||
      autoStart !== config.autoStart ||
      hostBridge !== config.hostBridge ||
      noatime !== config.noatime ||
      hostRuntime !== config.hostRuntime ||
      JSON.stringify(extraCharDevs) !== JSON.stringify(config.extraCharDevs ?? []) ||
      JSON.stringify(extraMounts) !== JSON.stringify(config.extraMounts ?? [])
    );
  }, [
    autoStart,
    config,
    extraCharDevs,
    extraMounts,
    hostBridge,
    hostRuntime,
    hostname,
    noatime,
    sshEnabled,
    sshPort,
  ]);

  const handleReset = () => {
    if (!config) return;
    setHostname(config.hostname);
    setSshEnabled(config.ssh.enabled);
    setSshPort(String(config.ssh.port));
    setSshPortError(null);
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setNoatime(config.noatime);
    setHostRuntime(config.hostRuntime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  };

  const handleSave = async () => {
    let parsedPort: number | undefined;
    if (sshEnabled) {
      const port = parseSshPort(sshPort);
      if (port === null) {
        setSshPortError('SSH 端口需为 1–65535 之间的有效整数');
        return;
      }
      parsedPort = port;
    }
    setSshPortError(null);
    await onSaveConfig({
      hostname: hostname.trim() || undefined,
      sshEnabled,
      ...(parsedPort !== undefined ? { sshPort: parsedPort } : {}),
      autoStart,
      hostBridge,
      noatime,
      hostRuntime,
      extraCharDevs,
      extraMounts,
    });
  };

  return (
    <div className="flex flex-col gap-3">
      {isDirty ? (
        <div className="sticky top-2 z-20 flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[color-mix(in_oklab,var(--accent)_45%,transparent)] bg-[var(--surface-card)] px-3 py-2 shadow-sm backdrop-blur-md">
          <div className="flex items-center gap-2">
            <span className="size-2 shrink-0 rounded-full bg-[var(--warning)]" />
            <span className="text-xs font-medium text-[var(--text-primary)]">
              有未保存修改 · 保存后重启容器生效
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <Button size="sm" variant="ghost" disabled={busy} onClick={handleReset}>
              <RotateCcw className="size-3.5" />
              还原
            </Button>
            <Button
              size="sm"
              variant="primary"
              disabled={busy || !config}
              onClick={() => void handleSave()}
            >
              <Save className="size-3.5" />
              保存配置
            </Button>
          </div>
        </div>
      ) : null}

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
                variant={isDirty ? 'primary' : 'secondary'}
                disabled={busy || !config || !isDirty}
                onClick={() => void handleSave()}
              >
                <Save className="size-3.5" />
                {isDirty ? '保存修改' : '已同步'}
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
                  disabled={!sshEnabled}
                  value={sshPort}
                  aria-label="SSH 端口"
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
              title="启用 Dropbear SSH 服务"
              description="仅公钥免密登录（纯容器未装 dropbear-bin 时请保持关闭）"
              control={
                <Switch
                  checked={sshEnabled}
                  onCheckedChange={(next) => {
                    setSshEnabled(next);
                    setSshPortError(null);
                    if (next && parseSshPort(sshPort) === null) {
                      setSshPort(String(STRATA_PROTOCOL.DEFAULT_SSH_PORT));
                    }
                  }}
                  aria-label="启用 Dropbear SSH 服务"
                />
              }
            />

            <div className="pochita-divider" />
            <SettingRow
              variant="plain"
              title="开机自启"
              description="后台服务启动时自动挂载容器并开启 SSH"
              control={
                <Switch checked={autoStart} onCheckedChange={setAutoStart} aria-label="开机自启" />
              }
            />
            <div className="pochita-divider" />
            <SettingRow
              variant="plain"
              title="Android 宿主机桥接 (hostBridge)"
              description="允许容器内通过 host-exec 调用 Android 宿主机调试能力"
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
              title="减少闪存写入 (noatime)"
              description="挂载镜像时禁用访问时间更新，提升读写性能并延长闪存寿命"
              control={
                <Switch checked={noatime} onCheckedChange={setNoatime} aria-label="减少闪存写入" />
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
            hasUnsavedChanges={isDirty}
            onSave={() => void handleSave()}
          />
        </div>

        <div className="flex flex-col gap-3 md:col-span-6">
          <SshKeysCard
            sshKeys={sshKeys}
            busy={busy}
            onAddSshKey={onAddSshKey}
            onRemoveSshKey={onRemoveSshKey}
          />
        </div>
      </div>
    </div>
  );
}
