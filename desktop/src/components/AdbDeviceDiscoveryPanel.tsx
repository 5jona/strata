import { Badge, Button, Input, StatusBanner } from '@gojona/ui';
import { RefreshCw } from 'lucide-react';
import type { AdbDevice } from '../types/device.ts';

export interface AdbDeviceDiscoveryPanelProps {
  adbPort: string;
  onAdbPortChange: (value: string) => void;
  adbDevices: AdbDevice[];
  scanningAdb: boolean;
  onScanAdb: () => Promise<void>;
  onConnectAdb: (adb: AdbDevice, remotePort: number) => Promise<void>;
  onClose: () => void;
}

/** ADB auto-discovery tab panel inside `AddDeviceModal`. */
export function AdbDeviceDiscoveryPanel({
  adbPort,
  onAdbPortChange,
  adbDevices,
  scanningAdb,
  onScanAdb,
  onConnectAdb,
  onClose,
}: AdbDeviceDiscoveryPanelProps) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
          <span>设备端服务端口：</span>
          <Input
            mono
            className="w-24"
            value={adbPort}
            onChange={(e) => onAdbPortChange(e.target.value)}
            aria-label="ADB 远端服务端口"
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
          description="请确认手机已开启 USB 调试或无线调试，并检查电脑端已正确配置 adb。"
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
  );
}
