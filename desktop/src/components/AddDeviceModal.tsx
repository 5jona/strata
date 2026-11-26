import { Button, Card, Input, SegmentedControl, StatusBanner } from '@gojona/ui';
import { Smartphone, Wifi, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { AdbDevice, DeviceProfile, EndpointProbeResult } from '../types/device.ts';
import { AdbDeviceDiscoveryPanel } from './AdbDeviceDiscoveryPanel.tsx';

export interface AddDeviceModalProps {
  open: boolean;
  editingDevice?: DeviceProfile | null;
  adbDevices: AdbDevice[];
  scanningAdb: boolean;
  onClose: () => void;
  onScanAdb: () => Promise<void>;
  onConnectAdb: (adb: AdbDevice, remotePort: number) => Promise<void>;
  onSaveManual: (profile: DeviceProfile) => Promise<void>;
}

/**
 * Multi-device onboarding and profile editor modal supporting ADB discovery/forwarding
 * and manual LAN/SSH endpoints with pairing token entry.
 */
export function AddDeviceModal({
  open,
  editingDevice = null,
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

  useEffect(() => {
    if (!open) return;
    setProbeResult(null);
    if (editingDevice) {
      setMode('manual');
      setLabel(editingDevice.label);
      setHost(editingDevice.host);
      setPort(String(editingDevice.port));
      setToken(editingDevice.token ?? '');
      setSshPort(String(editingDevice.sshPort ?? 2222));
      setKind(editingDevice.kind === 'ssh' ? 'ssh' : 'lan');
    } else {
      setMode('adb');
      setLabel('');
      setHost('192.168.1.');
      setPort('7681');
      setToken('');
      setSshPort('2222');
      setKind('lan');
    }
  }, [editingDevice, open]);

  if (!open) return null;

  const isEditing = editingDevice !== null;
  const isEditingAdbLike = editingDevice?.kind === 'adb' || editingDevice?.kind === 'mock';

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

  const handleSaveProfile = async () => {
    const cleanHost = host.trim() || '127.0.0.1';
    const cleanLabel = label.trim() || cleanHost;
    if (!isEditingAdbLike && !host.trim()) return;
    setSubmitting(true);
    try {
      const trimmedToken = token.trim();
      const profile: DeviceProfile = editingDevice
        ? {
            ...editingDevice,
            label: cleanLabel,
            kind: isEditingAdbLike ? editingDevice.kind : kind,
            host: isEditingAdbLike ? editingDevice.host : cleanHost,
            port: Number(port) || 7681,
            sshUser: editingDevice.sshUser ?? 'root',
            sshPort: Number(sshPort) || 2222,
            ...(trimmedToken ? { token: trimmedToken } : { token: undefined }),
          }
        : {
            id: `${kind}-${cleanHost.replace(/[^a-zA-Z0-9._-]/g, '-')}-${port}`,
            label: cleanLabel,
            kind,
            host: cleanHost,
            port: Number(port) || 7681,
            ...(trimmedToken ? { token: trimmedToken } : {}),
            sshUser: 'root',
            sshPort: Number(sshPort) || 2222,
          };
      await onSaveManual(profile);
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
      aria-label={isEditing ? '编辑设备配置' : '添加或扫描 Android 设备'}
    >
      <Card className="flex w-full max-w-lg flex-col gap-4 p-5">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold text-[var(--text-primary)]">
              {isEditing ? `编辑设备 · ${editingDevice.label}` : '连接 Android 设备'}
            </h2>
            <p className="text-xs text-[var(--text-secondary)]">
              {isEditing
                ? '更新连接端口或补充配对密钥 (Token)，保存后立即重新连接'
                : '请先在手机 KernelSU 安装并激活 Strata 模块，再通过 USB ADB 或局域网配对连接'}
            </p>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label="关闭弹窗" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>

        {!isEditing && (
          <SegmentedControl
            value={mode}
            onValueChange={setMode}
            fullWidth
            options={[
              { value: 'adb', label: 'ADB 自动发现', icon: <Smartphone className="size-3.5" /> },
              { value: 'manual', label: '手动局域网 / SSH', icon: <Wifi className="size-3.5" /> },
            ]}
          />
        )}

        {!isEditing && mode === 'adb' ? (
          <AdbDeviceDiscoveryPanel
            adbPort={adbPort}
            onAdbPortChange={setAdbPort}
            adbDevices={adbDevices}
            scanningAdb={scanningAdb}
            onScanAdb={onScanAdb}
            onConnectAdb={onConnectAdb}
            onClose={onClose}
          />
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
              {!isEditingAdbLike ? (
                <div className="flex flex-col gap-1">
                  <span className="text-xs text-[var(--text-secondary)]">连接方式</span>
                  <SegmentedControl
                    value={kind}
                    onValueChange={setKind}
                    size="sm"
                    fullWidth
                    options={[
                      { value: 'lan', label: '局域网直连' },
                      { value: 'ssh', label: 'SSH 转发' },
                    ]}
                  />
                </div>
              ) : (
                <div className="flex flex-col gap-1">
                  <span className="text-xs text-[var(--text-secondary)]">ADB 设备序列号</span>
                  <Input mono readOnly value={editingDevice?.serial ?? editingDevice?.host ?? ''} />
                </div>
              )}
            </div>
            <div className="grid grid-cols-3 gap-2.5">
              <div className="col-span-2 flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">主机 IP / 地址</span>
                <Input
                  mono
                  readOnly={isEditingAdbLike}
                  value={host}
                  placeholder="192.168.1.100 或 127.0.0.1"
                  onChange={(e) => setHost(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">服务端口</span>
                <Input mono value={port} onChange={(e) => setPort(e.target.value)} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-2.5">
              <div className="col-span-2 flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">配对密钥 (Token)</span>
                <Input
                  mono
                  type="password"
                  value={token}
                  placeholder="粘贴手机端显示的配对密钥"
                  onChange={(e) => setToken(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-xs text-[var(--text-secondary)]">默认 SSH 端口</span>
                <Input mono value={sshPort} onChange={(e) => setSshPort(e.target.value)} />
              </div>
            </div>
            <p className="text-[11px] text-[var(--text-secondary)]">
              请在手机 KernelSU → Strata「系统设置 → 电脑端配对连接」中开启局域网配对并复制密钥（USB
              ADB 连接通常会自动读取）。
            </p>
            {probeResult && (
              <StatusBanner
                tone={probeResult.reachable && probeResult.authenticated ? 'success' : 'warning'}
                title={
                  probeResult.reachable
                    ? probeResult.authenticated
                      ? `连通正常 (${probeResult.latencyMs}ms · 服务版本 ${probeResult.version ?? ''})`
                      : `端口可达 (${probeResult.latencyMs}ms)，但配对密钥鉴权未通过`
                    : '无法连接目标设备'
                }
                description={probeResult.error}
              />
            )}
            <div className="flex items-center justify-end gap-2 pt-1">
              {!isEditingAdbLike && (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={probing}
                  onClick={() => void handleProbe()}
                >
                  {probing ? '探测中…' : '测试连通性'}
                </Button>
              )}
              <Button
                size="sm"
                variant="primary"
                disabled={submitting || (!isEditingAdbLike && !host.trim())}
                onClick={() => void handleSaveProfile()}
              >
                {isEditing ? '保存并重连' : '保存并连接'}
              </Button>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}
