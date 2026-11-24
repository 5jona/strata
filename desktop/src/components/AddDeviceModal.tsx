import { Badge, Button, Card, Input, SegmentedControl, StatusBanner } from '@gojona/ui';
import { RefreshCw, Smartphone, Wifi, X } from 'lucide-react';
import { useState } from 'react';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { AdbDevice, DeviceProfile, EndpointProbeResult } from '../types/device.ts';

export interface AddDeviceModalProps {
  open: boolean;
  adbDevices: AdbDevice[];
  scanningAdb: boolean;
  onClose: () => void;
  onScanAdb: () => Promise<void>;
  onConnectAdb: (adb: AdbDevice, remotePort: number) => Promise<void>;
  onSaveManual: (profile: DeviceProfile) => Promise<void>;
}

/**
 * Multi-device onboarding modal supporting automatic ADB discovery/forwarding
 * and manual LAN/SSH Bearer-token endpoints.
 */
export function AddDeviceModal({
  open,
  adbDevices,
  scanningAdb,
  onClose,
  onScanAdb,
  onConnectAdb,
  onSaveManual,
}: AddDeviceModalProps) {
  const [mode, setMode] = useState<'adb' | 'manual'>('adb');
  const [adbPort, setAdbPort] = useState('7681');
  const [label, setLabel] = useState('');
  const [host, setHost] = useState('192.168.1.');
  const [port, setPort] = useState('7681');
  const [token, setToken] = useState('');
  const [sshPort, setSshPort] = useState('2222');
  const [kind, setKind] = useState<'lan' | 'ssh'>('lan');
  const [probing, setProbing] = useState(false);
  const [probeResult, setProbeResult] = useState<EndpointProbeResult | null>(null);
  const [submitting, setSubmitting] = useState(false);

  if (!open) return null;

  const handleProbe = async () => {
    setProbing(true);
    try {
      const res = await tauriDeviceBridge.probeEndpoint(
        host.trim(),
        Number(port) || 7681,
        token.trim() || undefined,
      );
      setProbeResult(res);
    } finally {
      setProbing(false);
    }
  };

  const handleManualSubmit = async () => {
    const cleanHost = host.trim();
    const cleanLabel = label.trim() || cleanHost;
    if (!cleanHost) return;
    setSubmitting(true);
    try {
      const safeId = `${kind}-${cleanHost.replace(/[^a-zA-Z0-9._-]/g, '-')}-${port}`;
      await onSaveManual({
        id: safeId,
        label: cleanLabel,
        kind,
        host: cleanHost,
        port: Number(port) || 7681,
        ...(token.trim() ? { token: token.trim() } : {}),
        sshUser: 'root',
        sshPort: Number(sshPort) || 2222,
      });
      onClose();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-label="添加或扫描 Android 设备"
    >
      <Card className="flex w-full max-w-lg flex-col gap-4 p-5">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold text-[var(--text-primary)]">
              连接 Android 设备
            </h2>
            <p className="text-xs text-[var(--text-secondary)]">
              支持 USB / 无线 ADB 自动转发并读取 Token，或手动添加局域网 / SSH 隧道设备
            </p>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="关闭弹窗" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        <SegmentedControl
          value={mode}
          onValueChange={setMode}
          fullWidth
          options={[
            { value: 'adb', label: 'ADB 自动发现', icon: <Smartphone className="size-3.5" /> },
            { value: 'manual', label: '手动 LAN / SSH', icon: <Wifi className="size-3.5" /> },
          ]}
        />

        {mode === 'adb' ? (
          <div className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
                <span>设备端 API 端口：</span>
                <Input
                  mono
                  className="w-24"
                  value={adbPort}
                  onChange={(e) => setAdbPort(e.target.value)}
                  aria-label="ADB 远端 API 端口"
                />
              </div>
              <Button
                size="sm"
                variant="secondary"
                disabled={scanningAdb}
                onClick={() => void onScanAdb()}
              >
                <RefreshCw className={`size-3.5 ${scanningAdb ? 'animate-spin' : ''}`} />
                重新扫描
              </Button>
            </div>

            {adbDevices.length === 0 ? (
              <StatusBanner
                tone="info"
                title="未检测到在线 ADB 设备"
                description="请确认已开启 USB 调试或无线调试，并检查电脑 PATH 中已安装 adb。"
              />
            ) : (
              <div className="flex max-h-60 flex-col gap-2 overflow-y-auto strata-scroll">
                {adbDevices.map((adb) => {
                  const ready = adb.state === 'device';
                  return (
                    <div
                      key={adb.serial}
                      className="flex items-center justify-between gap-3 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm font-medium text-[var(--text-primary)]">
                            {adb.model ?? adb.serial}
                          </span>
                          <Badge variant={ready ? 'success' : 'warning'}>
                            {ready ? (adb.isWireless ? '无线 ADB' : 'USB ADB') : adb.state}
                          </Badge>
                        </div>
                        <div className="truncate font-mono text-[11px] text-[var(--text-secondary)]">
                          {adb.serial} {adb.device ? `· ${adb.device}` : ''}
                        </div>
                      </div>
                      <Button
                        size="sm"
                        variant="primary"
                        disabled={!ready}
                        onClick={async () => {
                          await onConnectAdb(adb, Number(adbPort) || 7681);
                          onClose();
                        }}
                      >
                        一键连接
                      </Button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="grid grid-cols-2 gap-2.5">
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">设备名称</span>
                <Input
                  value={label}
                  placeholder="例如：书房平板"
                  onChange={(e) => setLabel(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">连接方式</span>
                <SegmentedControl
                  value={kind}
                  onValueChange={setKind}
                  size="sm"
                  fullWidth
                  options={[
                    { value: 'lan', label: 'LAN 直连' },
                    { value: 'ssh', label: 'SSH 转发' },
                  ]}
                />
              </div>
            </div>

            <div className="grid grid-cols-3 gap-2.5">
              <div className="col-span-2 flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">主机 IP / 地址</span>
                <Input
                  mono
                  value={host}
                  placeholder="192.168.1.100 或 127.0.0.1"
                  onChange={(e) => setHost(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">API 端口</span>
                <Input mono value={port} onChange={(e) => setPort(e.target.value)} />
              </div>
            </div>

            <div className="grid grid-cols-3 gap-2.5">
              <div className="col-span-2 flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">Bearer Token</span>
                <Input
                  mono
                  type="password"
                  value={token}
                  placeholder="cat /data/adb/strata/run/web.token"
                  onChange={(e) => setToken(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">默认 SSH 端口</span>
                <Input mono value={sshPort} onChange={(e) => setSshPort(e.target.value)} />
              </div>
            </div>

            {probeResult && (
              <StatusBanner
                tone={probeResult.reachable && probeResult.authenticated ? 'success' : 'warning'}
                title={
                  probeResult.reachable
                    ? probeResult.authenticated
                      ? `连通正常 (${probeResult.latencyMs}ms · stratad ${probeResult.version ?? ''})`
                      : `端口可达 (${probeResult.latencyMs}ms)，但鉴权未通过`
                    : '无法连接目标端点'
                }
                description={probeResult.error}
              />
            )}

            <div className="flex items-center justify-end gap-2 pt-1">
              <Button
                size="sm"
                variant="secondary"
                disabled={probing}
                onClick={() => void handleProbe()}
              >
                {probing ? '探测中…' : '测试连通性'}
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={submitting || !host.trim()}
                onClick={() => void handleManualSubmit()}
              >
                保存并连接
              </Button>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}
