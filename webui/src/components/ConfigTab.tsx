import {
  Badge,
  Button,
  Card,
  Input,
  LogConsole,
  type LogEntry,
  SettingRow,
  Switch,
} from '@pochita/ui';
import { KeyRound, Plus, RefreshCw, Save } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { summarizeSshKey } from '../lib/strataFormat.ts';
import type { BindMount, ContainerConfig, ContainerConfigPatchParams } from '../types.ts';
import { DebugPassthroughCard } from './DebugPassthroughCard.tsx';
import { StrataTraceCard } from './StrataTraceCard.tsx';

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
  const [sshPort, setSshPort] = useState(String(config?.ssh.port ?? 2222));
  const [autoStart, setAutoStart] = useState(config?.autoStart ?? true);
  const [hostBridge, setHostBridge] = useState(config?.hostBridge ?? true);
  const [hostRuntime, setHostRuntime] = useState(config?.hostRuntime ?? false);
  const [extraCharDevs, setExtraCharDevs] = useState<string[]>(config?.extraCharDevs ?? []);
  const [extraMounts, setExtraMounts] = useState<BindMount[]>(config?.extraMounts ?? []);
  const [newKey, setNewKey] = useState('');

  useEffect(() => {
    if (!config) return;
    setHostname(config.hostname);
    setSshPort(String(config.ssh.port));
    setAutoStart(config.autoStart);
    setHostBridge(config.hostBridge);
    setHostRuntime(config.hostRuntime);
    setExtraCharDevs(config.extraCharDevs ?? []);
    setExtraMounts(config.extraMounts ?? []);
  }, [config]);

  const handleSave = async () => {
    const parsedPort = Number.parseInt(sshPort, 10);
    const patch: ContainerConfigPatchParams = {
      hostname: hostname.trim() || undefined,
      sshPort:
        Number.isFinite(parsedPort) && parsedPort > 0 && parsedPort <= 65535
          ? parsedPort
          : undefined,
      autoStart,
      hostBridge,
      hostRuntime,
      extraCharDevs,
      extraMounts,
    };
    await onSaveConfig(patch);
  };

  const handleAddKey = async () => {
    const trimmed = newKey.trim();
    if (!trimmed) return;
    const ok = await onAddSshKey(trimmed);
    if (ok) setNewKey('');
  };

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-[var(--text-secondary)]">
          Dropbear 初始化日志
        </span>
        <Button
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
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium text-[var(--text-primary)]">容器参数</span>
          <Button
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
              onFocus={() => onImeFocusChange(true)}
              onBlur={() => onImeFocusChange(false)}
              onChange={(e) => setSshPort(e.target.value)}
            />
          </div>
        </div>

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

      <Card className="flex flex-col gap-2.5 p-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <KeyRound className="size-3.5 text-[var(--accent)]" />
            <span className="text-xs font-medium text-[var(--text-primary)]">SSH 授权公钥</span>
          </div>
          <Badge variant="neutral">{sshKeys.length} 条</Badge>
        </div>

        <div className="flex flex-col gap-1.5">
          {sshKeys.length === 0 ? (
            <p className="text-[11px] text-[var(--text-secondary)]">
              尚未配置公钥（Dropbear 仅允许公钥登录）。
            </p>
          ) : (
            sshKeys.map((line) => {
              const summary = summarizeSshKey(line);
              return (
                <div
                  key={line}
                  className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5"
                >
                  <span className="truncate font-mono text-[11px] text-[var(--text-primary)]">
                    {summary.label}
                  </span>
                  <Badge variant="accent">{summary.algo}</Badge>
                </div>
              );
            })
          )}
        </div>

        <div className="flex items-center gap-1.5">
          <Input
            mono
            value={newKey}
            placeholder="粘贴 ssh-ed25519 / ssh-rsa 公钥…"
            aria-label="新增 SSH 公钥输入框"
            onFocus={() => onImeFocusChange(true)}
            onBlur={() => onImeFocusChange(false)}
            onChange={(e) => setNewKey(e.target.value)}
          />
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || newKey.trim().length === 0}
            aria-label="追加 SSH 公钥"
            onClick={() => void handleAddKey()}
          >
            <Plus className="size-3.5" />
            添加
          </Button>
        </div>
      </Card>

      <StrataTraceCard onCopyLog={onCopyLog} />
    </div>
  );
}
