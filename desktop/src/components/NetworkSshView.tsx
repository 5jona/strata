import {
  Badge,
  Button,
  Card,
  copyTextWithFallback,
  Input,
  SettingRow,
  StatusBanner,
  Switch,
  useToast,
} from '@gojona/ui';
import { Code2, Copy, KeyRound, Plus, RefreshCw, ShieldAlert, Trash2, Wifi } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { formatSshConfigSnippet, formatVsCodeRemoteUri, summarizeSshKey } from '../lib/format.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile } from '../types/device.ts';
import type { ContainerStatusSnapshot, WebStatusResult } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';

export interface NetworkSshViewProps {
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  webStatus: WebStatusResult | null;
  onRefresh: () => Promise<void>;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Network, SSH key management, `~/.ssh/config` generator, VS Code Remote-SSH launcher,
 * ADB forward helper, and LAN Bearer-token security controls.
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
  const [rotatedToken, setRotatedToken] = useState<string | null>(null);
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

  const sshTarget = {
    user: device?.sshUser || 'root',
    host:
      activeContainer?.primaryIpv4 && activeContainer.primaryIpv4 !== '0.0.0.0'
        ? activeContainer.primaryIpv4
        : (device?.host ?? '127.0.0.1'),
    port: activeContainer?.sshPort ?? 2222,
    remotePath: '/root',
    alias: `strata-${activeContainer?.id ?? 'main'}`,
  };

  const sshConfigText = formatSshConfigSnippet(sshTarget);
  const vscodeUri = formatVsCodeRemoteUri(sshTarget);
  const adbForwardCmd = `adb forward tcp:${webStatus?.port ?? 7681} tcp:${webStatus?.port ?? 7681} && adb forward tcp:${sshTarget.port} tcp:${sshTarget.port}`;

  const copyText = (text: string, label: string) => {
    void copyTextWithFallback(text).then((ok) =>
      pushToast(ok ? `已复制 ${label}` : '复制失败', ok ? 'success' : 'warning'),
    );
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">网络与 SSH 配置</h1>
          <p className="text-xs text-[var(--text-secondary)]">
            管理 Dropbear 公钥授权、VS Code Remote-SSH 一键连接、ADB 隧道与局域网 API 鉴权
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
            VS Code Remote-SSH 与 ADB 端口转发
          </span>
          <div className="flex items-center gap-2">
            <Input mono readOnly value={vscodeUri} />
            <Button
              size="sm"
              variant="primary"
              onClick={() =>
                void tauriDeviceBridge
                  .openVsCodeRemote(sshTarget)
                  .then(() => pushToast('已拉起 VS Code Remote-SSH', 'info'))
                  .catch((e) => pushToast(String(e), 'danger'))
              }
            >
              <Code2 className="size-3.5" />
              打开
            </Button>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">USB 无 Wi-Fi 转发命令</span>
            <div className="flex items-center gap-2">
              <Input mono readOnly value={adbForwardCmd} />
              <Button
                size="sm"
                variant="secondary"
                onClick={() => copyText(adbForwardCmd, 'ADB 转发命令')}
              >
                <Copy className="size-3.5" />
                复制
              </Button>
            </div>
          </div>
        </Card>
      </div>

      {/* Embedded API & LAN Access Security */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Wifi className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              设备端控制面 API 与 Bearer Token
            </span>
          </div>
          {webStatus && (
            <Badge variant={webStatus.lanEnabled ? 'warning' : 'success'}>
              {webStatus.listenAddr}:{webStatus.port}
            </Badge>
          )}
        </div>

        <SettingRow
          variant="plain"
          title="允许局域网 (RFC1918) 访问"
          description="关闭时仅监听 127.0.0.1（通过 ADB 隧道访问）；开启后允许内网 IP 携带 Bearer Token 直连"
          control={
            <Switch
              checked={Boolean(webStatus?.lanEnabled)}
              disabled={busy || !transport}
              onCheckedChange={async (checked) => {
                if (!transport) return;
                setBusy(true);
                await runAction('更新局域网访问策略', () =>
                  transport.invoke(STRATA_METHODS.WEB_CONFIGURE, { lanEnabled: checked }),
                );
                await onRefresh();
                pushToast(checked ? '已开启 RFC1918 局域网访问' : '已切换为仅本机回环监听', 'info');
                setBusy(false);
              }}
            />
          }
        />

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-hairline)] pt-3">
          <div className="text-xs text-[var(--text-secondary)]">
            Token 文件路径：
            <span className="font-mono">
              {webStatus?.tokenPath ?? '/data/adb/strata/run/web.token'}
            </span>
          </div>
          <Button
            size="sm"
            variant="danger"
            disabled={busy || !transport}
            onClick={async () => {
              if (!transport) return;
              setBusy(true);
              const res = await runAction('轮换 Bearer Token', () =>
                transport.invoke(STRATA_METHODS.WEB_ROTATE_TOKEN),
              );
              if (res) {
                setRotatedToken(res.token);
                pushToast('Token 已轮换，旧会话已吊销，请及时保存新 Token', 'warning');
              }
              setBusy(false);
            }}
          >
            <RefreshCw className="size-3.5" />
            轮换 Bearer Token
          </Button>
        </div>

        {rotatedToken && (
          <StatusBanner
            tone="warning"
            title="新生成的 Bearer Token（仅显示一次）"
            description={rotatedToken}
            action={
              <Button
                size="sm"
                variant="secondary"
                onClick={() => copyText(rotatedToken, '新 Bearer Token')}
              >
                <ShieldAlert className="size-3.5" />
                复制 Token
              </Button>
            }
          />
        )}
      </Card>
    </div>
  );
}
