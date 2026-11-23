import { Button, Card, Input, LogConsole, type LogEntry, SettingRow, Switch } from '@gojona/ui';
import { RefreshCw, Save } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { parseSshPort } from '../lib/strataFormat.ts';
import {
  type BindMount,
  type ContainerConfig,
  type ContainerConfigPatchParams,
  STRATA_PROTOCOL,
} from '../types.ts';
import { DebugPassthroughCard } from './DebugPassthroughCard.tsx';
import { SshKeysCard } from './SshKeysCard.tsx';
import { StrataTraceCard } from './StrataTraceCard.tsx';

/** Props accepted by `ConfigTab`. */
export interface ConfigTabProps {
  config: ContainerConfig | null;
  dropbearLogs: readonly LogEntry[];
  sshKeys: readonly string[];
  busy: boolean;
  onRefresh: () => Promise<void>;
  onSaveConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  onAddSshKey: (publicKey: string) => Promise<boolean>;
  onCopyLog: (text: string) => Promise<boolean>;
  onImeFocusChange: (focused: boolean) => void;
}

/** Configuration, Dropbear log, SSH authorized-key, and bridge trace management tab. */
export function ConfigTab({
  config,
  dropbearLogs,
  sshKeys,
  busy,
  onRefresh,
  onSaveConfig,
  onAddSshKey,
  onCopyLog,
  onImeFocusChange,
}: ConfigTabProps) {
  const hostnameId = useId();
  const sshPortId = useId();
  const [hostname, setHostname] = useState(config?.hostname ?? '');
  const [sshPort, setSshPort] = useState(
    String(config?.ssh.port ?? STRATA_PROTOCOL.DEFAULT_SSH_PORT),
  );
  const [sshPortError, setSshPortError] = useState<string | null>(null);
  const [autoStart, setAutoStart] = useState(config?.autoStart ?? true);
  const [hostBridge, setHostBridge] = useState(config?.hostBridge ?? true);
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
    setHostRuntime(config.hostRuntime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config]);

  const handleSshPortChange = (value: string) => {
    setSshPort(value);
    if (sshPortError && parseSshPort(value) !== null) {
      setSshPortError(null);
    }
  };

  const handleSave = async () => {
    const parsedPort = parseSshPort(sshPort);
    if (parsedPort === null) {
      setSshPortError('SSH 端口需为 1–65535 之间的有效整数');
      return;
    }
    setSshPortError(null);
    const patch: ContainerConfigPatchParams = {
      hostname: hostname.trim() || undefined,
      sshPort: parsedPort,
      autoStart,
      hostBridge,
      hostRuntime,
      extraCharDevs,
      extraMounts,
    };
    await onSaveConfig(patch);
  };

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center justify-between gap-2">
        <span className="truncate text-xs font-medium text-[var(--text-secondary)]">
          Dropbear 初始化日志
        </span>
        <Button
          className="shrink-0"
          size="sm"
          variant="ghost"
          disabled={busy}
          aria-label="刷新容器日志和配置"
          onClick={() => void onRefresh()}
        >
          <RefreshCw className="size-3.5" />
          刷新
        </Button>
      </div>

      <LogConsole
        title="Dropbear 日志"
        entries={dropbearLogs}
        searchable
        filterable
        onCopy={onCopyLog}
        maxHeightClassName="max-h-40"
      />

      <Card className="flex flex-col gap-2.5 p-3">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-xs font-medium text-[var(--text-primary)]">容器参数</span>
          <Button
            className="shrink-0"
            size="sm"
            variant="primary"
            disabled={busy || !config}
            aria-label="保存容器配置"
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
              onFocus={() => onImeFocusChange(true)}
              onBlur={() => onImeFocusChange(false)}
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
              onFocus={() => onImeFocusChange(true)}
              onBlur={() => onImeFocusChange(false)}
              onChange={(e) => handleSshPortChange(e.target.value)}
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

      <SshKeysCard
        sshKeys={sshKeys}
        busy={busy}
        onAddSshKey={onAddSshKey}
        onImeFocusChange={onImeFocusChange}
      />

      <StrataTraceCard onCopyLog={onCopyLog} />
    </div>
  );
}
