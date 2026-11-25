import {
  Badge,
  Button,
  Card,
  LogConsole,
  type LogEntry,
  SegmentedControl,
  ThemeSwitcher,
} from '@gojona/ui';
import { Cpu, RefreshCw } from 'lucide-react';
import { useState } from 'react';
import type { KernelCapabilitiesResult } from '../types.ts';

/** Props accepted by `KsuDiagnosticsTab`. */
export interface KsuDiagnosticsTabProps {
  daemonLogs: readonly LogEntry[];
  dropbearLogs: readonly LogEntry[];
  capabilities: KernelCapabilitiesResult | null;
  busy: boolean;
  onRefresh: () => Promise<void>;
  onCopyLog: (text: string) => Promise<boolean>;
}

const CAPABILITY_ITEMS: ReadonlyArray<{
  key: keyof Omit<KernelCapabilitiesResult, 'kernelRelease'>;
  label: string;
}> = [
  { key: 'cgroupV2', label: 'cgroup v2' },
  { key: 'cgroupPids', label: 'cgroup.pids' },
  { key: 'cgroupBpf', label: 'cgroup BPF' },
  { key: 'netNs', label: 'NET_NS' },
  { key: 'overlayFs', label: 'OverlayFS' },
  { key: 'fuseFs', label: 'FUSE' },
  { key: 'ioUring', label: 'io_uring' },
  { key: 'pidNs', label: 'PID_NS' },
  { key: 'ipcNs', label: 'IPC_NS' },
  { key: 'sysvIpc', label: 'SYSVIPC' },
  { key: 'userNs', label: 'USER_NS' },
  { key: 'devtmpfs', label: 'devtmpfs' },
  { key: 'kvm', label: 'KVM' },
  { key: 'systemdReady', label: 'Systemd 就绪' },
  { key: 'dockerReady', label: 'Docker 就绪' },
];

/** Tab 4: daemon/Dropbear log inspector, kernel capability matrix, and theme switcher. */
export function KsuDiagnosticsTab({
  daemonLogs,
  dropbearLogs,
  capabilities,
  busy,
  onRefresh,
  onCopyLog,
}: KsuDiagnosticsTabProps) {
  const [logSource, setLogSource] = useState<'daemon' | 'dropbear'>('daemon');
  const activeEntries = logSource === 'daemon' ? daemonLogs : dropbearLogs;

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
      <div className="flex flex-col gap-3 md:col-span-7">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SegmentedControl
            value={logSource}
            onValueChange={(v) => setLogSource(v === 'dropbear' ? 'dropbear' : 'daemon')}
            options={[
              { value: 'daemon', label: 'stratad 日志' },
              { value: 'dropbear', label: 'Dropbear 日志' },
            ]}
          />
          <Button size="sm" variant="secondary" disabled={busy} onClick={() => void onRefresh()}>
            <RefreshCw className="size-3.5" />
            刷新日志与环境
          </Button>
        </div>

        <LogConsole
          title={logSource === 'daemon' ? 'stratad 守护进程日志' : '容器 Dropbear SSH 日志'}
          entries={activeEntries}
          searchable
          filterable
          onCopy={onCopyLog}
          maxHeightClassName="max-h-72"
        />
      </div>

      <div className="flex flex-col gap-3 md:col-span-5">
        <Card className="flex flex-col gap-2.5 p-3">
          <div className="flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-1.5">
              <Cpu className="size-3.5 shrink-0 text-[var(--accent)]" />
              <span className="truncate text-xs font-medium text-[var(--text-primary)]">
                内核兼容能力探测
              </span>
            </div>
            {capabilities ? (
              <Badge className="shrink-0" variant="accent">
                {capabilities.kernelRelease}
              </Badge>
            ) : null}
          </div>

          {capabilities ? (
            <div className="flex flex-wrap gap-1.5">
              {CAPABILITY_ITEMS.map(({ key, label }) => {
                const supported = capabilities[key];
                return (
                  <Badge key={key} variant={supported ? 'success' : 'neutral'} dot>
                    {label}
                  </Badge>
                );
              })}
            </div>
          ) : (
            <p className="text-[11px] text-[var(--text-secondary)]">暂无内核能力探测数据</p>
          )}
        </Card>

        <Card className="flex flex-col gap-2 p-3">
          <span className="text-xs font-medium text-[var(--text-primary)]">界面主题</span>
          <ThemeSwitcher compact />
        </Card>
      </div>
    </div>
  );
}
