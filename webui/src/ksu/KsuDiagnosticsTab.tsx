import { Badge, Button, Card, LogConsole, type LogEntry, SegmentedControl } from '@gojona/ui';
import { CheckCircle2, Cpu, RefreshCw, XCircle } from 'lucide-react';
import { useState } from 'react';
import type { KernelCapabilitiesResult } from '../types.ts';

/** Props accepted by `KsuDiagnosticsTab`. */
export interface KsuDiagnosticsTabProps {
  daemonLogs: readonly LogEntry[];
  dropbearLogs: readonly LogEntry[];
  capabilities: KernelCapabilitiesResult | null;
  hasContainer?: boolean;
  busy: boolean;
  onRefresh: () => Promise<void>;
  onCopyLog: (text: string) => Promise<boolean>;
}

const CAPABILITY_ITEMS: ReadonlyArray<{
  key: keyof Omit<KernelCapabilitiesResult, 'kernelRelease'>;
  label: string;
  hint: string;
}> = [
  { key: 'cgroupV2', label: 'cgroup v2 资源控制', hint: '进程组隔离与安全排空' },
  { key: 'cgroupPids', label: 'cgroup pids 控制器', hint: '限制容器最大进程数，防 fork 炸弹' },
  { key: 'cgroupBpf', label: 'cgroup BPF', hint: '内核级网络与行为追踪' },
  { key: 'netNs', label: 'Network 命名空间', hint: '独立网络栈隔离' },
  { key: 'overlayFs', label: 'OverlayFS', hint: '联合分层挂载' },
  { key: 'fuseFs', label: 'FUSE 文件系统', hint: '支持用户态虚拟挂载' },
  { key: 'ioUring', label: 'io_uring', hint: '高吞吐异步 I/O 加速' },
  { key: 'pidNs', label: 'PID 命名空间', hint: '进程号隔离（原厂 GKI 常关闭，不影响使用）' },
  { key: 'ipcNs', label: 'IPC 命名空间', hint: 'POSIX 消息队列隔离' },
  { key: 'sysvIpc', label: 'SYSVIPC 共享内存', hint: 'PostgreSQL / 桌面软件常用' },
  { key: 'userNs', label: 'User 命名空间', hint: '非特权用户映射（原厂内核常关闭）' },
  { key: 'devtmpfs', label: 'devtmpfs 动态设备树', hint: '完整 systemd / udev 设备管理' },
  { key: 'kvm', label: 'KVM 硬件虚拟化', hint: '嵌套虚拟机加速' },
  { key: 'systemdReady', label: 'systemd 就绪', hint: '完整运行 systemd 初始化栈' },
  { key: 'dockerReady', label: 'Docker 引擎就绪', hint: '直接运行 OCI 容器引擎' },
];

/** Tab 4: daemon/Dropbear log inspector and kernel capability matrix. */
export function KsuDiagnosticsTab({
  daemonLogs,
  dropbearLogs,
  capabilities,
  hasContainer = true,
  busy,
  onRefresh,
  onCopyLog,
}: KsuDiagnosticsTabProps) {
  const [logSource, setLogSource] = useState<'daemon' | 'dropbear'>('daemon');
  const activeSource = hasContainer ? logSource : 'daemon';
  const activeEntries = activeSource === 'daemon' ? daemonLogs : dropbearLogs;

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
      <div className="flex flex-col gap-3 md:col-span-7">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SegmentedControl
            value={activeSource}
            onValueChange={(v) => setLogSource(v === 'dropbear' ? 'dropbear' : 'daemon')}
            options={
              hasContainer
                ? [
                    { value: 'daemon', label: '后台服务日志' },
                    { value: 'dropbear', label: '容器 SSH 日志' },
                  ]
                : [{ value: 'daemon', label: '后台服务日志' }]
            }
          />
          <Button size="sm" variant="secondary" disabled={busy} onClick={() => void onRefresh()}>
            <RefreshCw className="size-3.5" />
            刷新日志与内核能力
          </Button>
        </div>

        <LogConsole
          title={activeSource === 'daemon' ? 'Strata 后台服务日志' : '容器 SSH (Dropbear) 日志'}
          entries={activeEntries}
          searchable
          filterable
          onCopy={onCopyLog}
          maxHeightClassName="max-h-80"
        />
      </div>

      <div className="flex flex-col gap-3 md:col-span-5">
        <Card className="flex flex-col gap-2.5 p-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-1.5">
              <Cpu className="size-3.5 shrink-0 text-[var(--accent)]" />
              <span className="truncate text-xs font-medium text-[var(--text-primary)]">
                内核兼容能力探测
              </span>
            </div>
            {capabilities ? (
              <div className="flex flex-wrap items-center gap-1">
                <Badge className="shrink-0" variant="neutral">
                  {capabilities.kernelRelease}
                </Badge>
                <Badge
                  className="shrink-0"
                  variant={capabilities.dockerReady ? 'success' : 'accent'}
                >
                  {capabilities.dockerReady ? '完整内核环境' : '原厂 GKI 兼容模式'}
                </Badge>
              </div>
            ) : null}
          </div>

          {capabilities ? (
            <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2 md:grid-cols-1">
              {CAPABILITY_ITEMS.map(({ key, label, hint }) => {
                const supported = capabilities[key];
                return (
                  <div
                    key={key}
                    className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1.5"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-[11px] font-medium text-[var(--text-primary)]">
                        {label}
                      </div>
                      <div className="truncate text-[10px] text-[var(--text-secondary)]">
                        {hint}
                      </div>
                    </div>
                    {supported ? (
                      <CheckCircle2 className="size-3.5 shrink-0 text-[var(--success)]" />
                    ) : (
                      <XCircle className="size-3.5 shrink-0 text-[var(--text-secondary)] opacity-50" />
                    )}
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-[11px] text-[var(--text-secondary)]">暂无内核能力探测数据</p>
          )}
        </Card>
      </div>
    </div>
  );
}
