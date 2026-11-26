import { Badge, Button, Card, Input, SettingRow, StatusBanner, Switch, useToast } from '@gojona/ui';
import { Cable, Copy, Plus, RefreshCw, ShieldAlert, Trash2, Wifi } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile, PortForwardEntry } from '../types/device.ts';
import { STRATA_PATHS, type WebStatusResult } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';

export interface PortForwardAndApiCardProps {
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  activeContainerSshPort: number;
  webStatus: WebStatusResult | null;
  busy: boolean;
  setBusy: (busy: boolean) => void;
  onRefresh: () => Promise<void>;
  onCopyText: (text: string, label: string) => void;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * USB ADB container port-forwarding table + LAN Bearer-token control-plane security card.
 */
export function PortForwardAndApiCard({
  device,
  transport,
  activeContainerSshPort,
  webStatus,
  busy,
  setBusy,
  onRefresh,
  onCopyText,
  runAction,
}: PortForwardAndApiCardProps) {
  const { pushToast } = useToast();
  const [forwards, setForwards] = useState<PortForwardEntry[]>([]);
  const [remotePort, setRemotePort] = useState(String(activeContainerSshPort));
  const [localPort, setLocalPort] = useState('');
  const [rotatedToken, setRotatedToken] = useState<string | null>(null);

  useEffect(() => {
    setRemotePort(String(activeContainerSshPort));
  }, [activeContainerSshPort]);

  const serial = device?.serial?.trim() ?? '';
  const loadForwards = useCallback(async () => {
    if (!serial) {
      setForwards([]);
      return;
    }
    try {
      const list = await tauriDeviceBridge.listPortForwards(serial);
      setForwards(list);
    } catch {
      setForwards([]);
    }
  }, [serial]);

  useEffect(() => {
    void loadForwards();
  }, [loadForwards]);

  const handleCreateForward = async () => {
    if (!serial) return;
    const remote = Number(remotePort);
    const local = localPort.trim() ? Number(localPort) : undefined;
    if (!Number.isInteger(remote) || remote < 1 || remote > 65535) {
      pushToast('请输入有效的远程端口 (1-65535)', 'warning');
      return;
    }
    setBusy(true);
    const entry = await runAction('建立 ADB 端口转发', () =>
      tauriDeviceBridge.forwardDevicePort(serial, remote, local),
    );
    if (entry) {
      setLocalPort('');
      await loadForwards();
      pushToast(`已建立转发 127.0.0.1:${entry.localPort} → 容器:${entry.remotePort}`, 'success');
    }
    setBusy(false);
  };

  return (
    <div className="flex flex-col gap-4">
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Cable className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              USB ADB 端口转发
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            {serial ? (
              <Badge variant="accent">序列号 {serial}</Badge>
            ) : (
              <Badge variant="neutral">非 ADB 连接</Badge>
            )}
            <Button
              size="icon-sm"
              variant="ghost"
              disabled={!serial}
              aria-label="刷新端口转发列表"
              onClick={() => void loadForwards()}
            >
              <RefreshCw className="size-3.5" />
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Input
            mono
            className="w-32"
            value={remotePort}
            placeholder="容器端口"
            aria-label="容器目标端口"
            onChange={(e) => setRemotePort(e.target.value)}
          />
          <Input
            mono
            className="w-36"
            value={localPort}
            placeholder="本地端口 (自动)"
            aria-label="本地回环端口"
            onChange={(e) => setLocalPort(e.target.value)}
          />
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setRemotePort(String(activeContainerSshPort))}
          >
            SSH ({activeContainerSshPort})
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setRemotePort(String(webStatus?.port ?? 7681))}
          >
            API ({webStatus?.port ?? 7681})
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !serial}
            onClick={() => void handleCreateForward()}
          >
            <Plus className="size-3.5" />
            建立转发
          </Button>
        </div>

        {forwards.length === 0 ? (
          <p className="text-xs text-[var(--text-secondary)]">
            {serial
              ? '当前无活跃 ADB 端口转发。连接 VS Code 或外部终端时会自动按需建立 SSH 转发。'
              : '仅通过 USB / 无线 ADB 连接的设备支持自动建立 adb forward 隧道。'}
          </p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {forwards.map((item) => (
              <div
                key={`${item.serial}:${item.localPort}:${item.remotePort}`}
                className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2 text-xs"
              >
                <div className="flex items-center gap-2 font-mono text-[var(--text-primary)]">
                  <Badge variant="success">127.0.0.1:{item.localPort}</Badge>
                  <span>→</span>
                  <span>{item.remoteSpec}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    title="复制本地转发地址"
                    onClick={() => onCopyText(`127.0.0.1:${item.localPort}`, '转发地址')}
                  >
                    <Copy className="size-3.5" />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    disabled={busy}
                    aria-label={`移除转发 ${item.localSpec}`}
                    onClick={async () => {
                      setBusy(true);
                      await runAction('移除 ADB 端口转发', () =>
                        tauriDeviceBridge.removePortForward(item.serial, item.localPort),
                      );
                      await loadForwards();
                      setBusy(false);
                    }}
                  >
                    <Trash2 className="size-3.5 text-[var(--danger)]" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card className="flex flex-col gap-3 p-4">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Wifi className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              设备端远程配对与密钥 (Token)
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
          title="允许局域网 Wi-Fi 访问"
          description="关闭时仅通过 USB ADB 隧道访问；开启后允许同一局域网设备凭配对密钥直连"
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
                pushToast(
                  checked ? '已开启局域网 Wi-Fi 访问' : '已切换为仅本机与 ADB 隧道访问',
                  'info',
                );
                setBusy(false);
              }}
            />
          }
        />

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-hairline)] pt-3">
          <div className="text-xs text-[var(--text-secondary)]">
            密钥路径：
            <span className="font-mono">
              {webStatus?.tokenPath ?? STRATA_PATHS.DEFAULT_WEB_TOKEN_PATH}
            </span>
          </div>
          <Button
            size="sm"
            variant="danger"
            disabled={busy || !transport}
            onClick={async () => {
              if (!transport) return;
              setBusy(true);
              const res = await runAction('刷新配对密钥', () =>
                transport.invoke(STRATA_METHODS.WEB_ROTATE_TOKEN),
              );
              if (res) {
                setRotatedToken(res.token);
                pushToast('配对密钥已刷新，旧连接已断开，请及时保存新密钥', 'warning');
              }
              setBusy(false);
            }}
          >
            <RefreshCw className="size-3.5" />
            刷新配对密钥
          </Button>
        </div>

        {rotatedToken && (
          <StatusBanner
            tone="warning"
            title="新生成的配对密钥（仅显示一次）"
            description={rotatedToken}
            action={
              <Button
                size="sm"
                variant="secondary"
                onClick={() => onCopyText(rotatedToken, '新配对密钥')}
              >
                <ShieldAlert className="size-3.5" />
                复制密钥
              </Button>
            }
          />
        )}
      </Card>
    </div>
  );
}
