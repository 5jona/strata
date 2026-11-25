import { Badge, Button, Card, copyTextWithFallback, Input, useToast } from '@gojona/ui';
import { Code2, Copy, ExternalLink, KeyRound, Plus, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { formatSshConfigSnippet, formatVsCodeRemoteUri, summarizeSshKey } from '../lib/format.ts';
import { buildPreviewSshTarget, resolveLaunchSshTarget } from '../lib/sshTarget.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile } from '../types/device.ts';
import type { ContainerStatusSnapshot, WebStatusResult } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { PortForwardAndApiCard } from './PortForwardAndApiCard.tsx';

export interface NetworkSshViewProps {
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  webStatus: WebStatusResult | null;
  onRefresh: () => Promise<void>;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Network, SSH key management, `~/.ssh/config` generator, VS Code / external terminal SSH launcher,
 * ADB port forwarding table, and LAN Bearer-token security controls.
 */
export function NetworkSshView({
  device,
  transport,
  containers,
  webStatus,
  onRefresh,
  runAction,
}: NetworkSshViewProps) {
  const { pushToast } = useToast();
  const [selectedContainer, setSelectedContainer] = useState(containers[0]?.id ?? 'main');
  const [sshKeys, setSshKeys] = useState<string[]>([]);
  const [newKey, setNewKey] = useState('');
  const [busy, setBusy] = useState(false);

  const activeContainer =
    containers.find((c) => c.id === selectedContainer) ?? containers[0] ?? null;

  const loadKeys = useCallback(async () => {
    if (!transport || !activeContainer) return;
    try {
      const res = await transport.invoke(STRATA_METHODS.CONTAINER_SSH_KEY_LIST, {
        container: activeContainer.id,
      });
      setSshKeys(res.keys);
    } catch {
      setSshKeys([]);
    }
  }, [activeContainer, transport]);

  useEffect(() => {
    void loadKeys();
  }, [loadKeys]);

  const sshTarget = buildPreviewSshTarget(device, activeContainer);
  const sshConfigText = formatSshConfigSnippet(sshTarget);
  const vscodeUri = formatVsCodeRemoteUri(sshTarget);

  const copyText = (text: string, label: string) => {
    void copyTextWithFallback(text).then((ok) =>
      pushToast(ok ? `已复制 ${label}` : '复制失败', ok ? 'success' : 'warning'),
    );
  };

  const handleLaunchIde = (mode: 'vscode' | 'terminal') => {
    if (!activeContainer) return;
    void runAction(mode === 'vscode' ? '拉起 VS Code Remote-SSH' : '启动外部终端 SSH', async () => {
      const { target, forwardedLocalPort } = await resolveLaunchSshTarget(device, activeContainer);
      if (mode === 'vscode') {
        await tauriDeviceBridge.openVsCodeRemote(target);
        pushToast(
          forwardedLocalPort
            ? `已自动转发 SSH (127.0.0.1:${forwardedLocalPort}) 并拉起 VS Code`
            : '已拉起 VS Code Remote-SSH',
          'info',
        );
      } else {
        const prog = await tauriDeviceBridge.openNativeTerminal(target);
        pushToast(
          forwardedLocalPort
            ? `已自动转发 SSH (127.0.0.1:${forwardedLocalPort}) 并启动 ${prog}`
            : `已启动外部终端: ${prog}`,
          'info',
        );
      }
    });
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">网络与端口转发</h1>
          <p className="text-xs text-[var(--text-secondary)]">
            管理 Dropbear 公钥授权、VS Code / 外部终端 SSH 一键直连、USB ADB 端口转发与 API 鉴权
          </p>
        </div>
        <div className="flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-1">
          {containers.map((c) => (
            <Button
              key={c.id}
              size="sm"
              variant={activeContainer?.id === c.id ? 'pill' : 'ghost'}
              onClick={() => setSelectedContainer(c.id)}
            >
              {c.id}
            </Button>
          ))}
        </div>
      </div>

      {/* SSH Keys Management */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <KeyRound className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              SSH 授权公钥 · {activeContainer?.id ?? 'main'}
            </span>
          </div>
          <Badge variant="neutral">{sshKeys.length} 条</Badge>
        </div>

        {sshKeys.length === 0 ? (
          <p className="text-xs text-[var(--text-secondary)]">
            尚未配置公钥（Dropbear 仅允许公钥登录）。
          </p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {sshKeys.map((line) => {
              const info = summarizeSshKey(line);
              return (
                <div
                  key={line}
                  className="flex items-center justify-between gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2"
                >
                  <span className="truncate font-mono text-xs text-[var(--text-primary)]">
                    {info.label}
                  </span>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <Badge variant="accent">{info.algo}</Badge>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`移除公钥 ${info.label}`}
                      disabled={busy}
                      onClick={async () => {
                        if (!transport || !activeContainer) return;
                        setBusy(true);
                        await runAction('移除 SSH 公钥', () =>
                          transport.invoke(STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE, {
                            id: activeContainer.id,
                            fingerprintOrKey: line,
                          }),
                        );
                        await loadKeys();
                        setBusy(false);
                      }}
                    >
                      <Trash2 className="size-3.5 text-[var(--danger)]" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div className="flex items-center gap-2">
          <Input
            mono
            value={newKey}
            placeholder="粘贴 ssh-ed25519 / ssh-rsa 公钥…"
            onChange={(e) => setNewKey(e.target.value)}
          />
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !newKey.trim() || !transport || !activeContainer}
            onClick={async () => {
              if (!transport || !activeContainer) return;
              setBusy(true);
              const res = await runAction('添加 SSH 公钥', () =>
                transport.invoke(STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
                  container: activeContainer.id,
                  publicKey: newKey.trim(),
                }),
              );
              if (res) {
                setNewKey('');
                await loadKeys();
                pushToast('已追加 SSH 公钥', 'success');
              }
              setBusy(false);
            }}
          >
            <Plus className="size-3.5" />
            添加
          </Button>
        </div>
      </Card>

      {/* VS Code Remote-SSH & ~/.ssh/config */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card className="flex flex-col gap-3 p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              ~/.ssh/config 快捷配置
            </span>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => copyText(sshConfigText, '~/.ssh/config 片段')}
            >
              <Copy className="size-3.5" />
              复制配置
            </Button>
          </div>
          <pre className="overflow-x-auto rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3 font-mono text-xs text-[var(--text-primary)]">
            {sshConfigText}
          </pre>
        </Card>

        <Card className="flex flex-col gap-3 p-4">
          <span className="text-sm font-semibold text-[var(--text-primary)]">
            一键连 IDE 与外部终端
          </span>
          <div className="flex items-center gap-2">
            <Input mono readOnly value={vscodeUri} />
            <Button
              size="sm"
              variant="primary"
              disabled={!activeContainer}
              onClick={() => handleLaunchIde('vscode')}
            >
              <Code2 className="size-3.5" />
              VS Code
            </Button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={!activeContainer}
              onClick={() => handleLaunchIde('terminal')}
            >
              <ExternalLink className="size-3.5" />
              外部终端 SSH
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                copyText(`ssh -p ${sshTarget.port} ${sshTarget.user}@${sshTarget.host}`, 'SSH 命令')
              }
            >
              <Copy className="size-3.5" />
              复制 SSH 命令
            </Button>
          </div>
        </Card>
      </div>

      <PortForwardAndApiCard
        device={device}
        transport={transport}
        activeContainerSshPort={activeContainer?.sshPort ?? 2222}
        webStatus={webStatus}
        busy={busy}
        setBusy={setBusy}
        onRefresh={onRefresh}
        onCopyText={copyText}
        runAction={runAction}
      />
    </div>
  );
}
