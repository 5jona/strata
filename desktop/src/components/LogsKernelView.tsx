import { Badge, Button, Card, LogConsole, type LogEntry, SegmentedControl } from '@gojona/ui';
import { CheckCircle2, Cpu, RefreshCw, XCircle } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type { ContainerStatusSnapshot, KernelCapabilitiesResult } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';

export interface LogsKernelViewProps {
  transport: StrataTransport | null;
  capabilities: KernelCapabilitiesResult | null;
  containers: ContainerStatusSnapshot[];
}

interface CapItem {
  key: keyof Omit<KernelCapabilitiesResult, 'kernelRelease'>;
  label: string;
  tier: 'stock-gki' | 'tier2';
  hint: string;
}

const CAP_MATRIX: CapItem[] = [
  { key: 'pidNs', label: 'PID 命名空间', tier: 'stock-gki', hint: '进程隔离' },
  { key: 'ipcNs', label: 'IPC 命名空间', tier: 'stock-gki', hint: 'POSIX 消息隔离' },
  { key: 'netNs', label: 'Network 命名空间', tier: 'stock-gki', hint: '独立网络栈' },
  { key: 'userNs', label: 'User 命名空间', tier: 'stock-gki', hint: '非特权映射' },
  { key: 'overlayFs', label: 'OverlayFS', tier: 'stock-gki', hint: '联合挂载层' },
  { key: 'fuseFs', label: 'FUSE 文件系统', tier: 'stock-gki', hint: '用户态挂载' },
  { key: 'cgroupV2', label: 'cgroup v2', tier: 'stock-gki', hint: '资源控制与排空' },
  { key: 'cgroupBpf', label: 'cgroup BPF', tier: 'stock-gki', hint: 'L5 eBPF 追踪' },
  { key: 'ioUring', label: 'io_uring', tier: 'stock-gki', hint: '异步 I/O 加速' },
  { key: 'sysvIpc', label: 'SYSVIPC', tier: 'tier2', hint: 'PostgreSQL / Chromium 共享内存' },
  { key: 'cgroupPids', label: 'cgroup pids 控制器', tier: 'tier2', hint: '防止 fork 炸弹' },
  { key: 'devtmpfs', label: 'devtmpfs', tier: 'tier2', hint: 'systemd / udev 设备树' },
  { key: 'kvm', label: 'KVM 硬件虚拟化', tier: 'tier2', hint: '嵌套虚拟机加速' },
  { key: 'systemdReady', label: 'systemd 就绪', tier: 'tier2', hint: '完整系统初始化栈' },
  { key: 'dockerReady', label: 'Docker 引擎就绪', tier: 'tier2', hint: 'OCI 守护进程支持' },
];

/**
 * Diagnostics view rendering `daemon.capabilities` Kernel Capability Matrix
 * (Stock GKI vs Tier 2) and live `stratad` / container log streams.
 */
export function LogsKernelView({ transport, capabilities, containers }: LogsKernelViewProps) {
  const [logSource, setLogSource] = useState<'daemon' | string>('daemon');
  const [lines, setLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);

  const loadLogs = useCallback(async () => {
    if (!transport) return;
    setLoading(true);
    try {
      if (logSource === 'daemon') {
        const res = await transport.invoke(STRATA_METHODS.DAEMON_LOGS, { lines: 250 });
        setLines(res.lines);
      } else {
        const res = await transport.invoke(STRATA_METHODS.CONTAINER_LOGS, {
          container: logSource,
          lines: 250,
        });
        setLines(res.lines);
      }
    } catch {
      setLines([]);
    } finally {
      setLoading(false);
    }
  }, [logSource, transport]);

  useEffect(() => {
    void loadLogs();
  }, [loadLogs]);

  const entries: LogEntry[] = lines.map((line, i) => ({
    id: `${logSource}-${i}`,
    timestamp: String(i + 1).padStart(3, '0'),
    level: /\[ERR|ERROR|panic|fail/i.test(line)
      ? 'error'
      : /\[WARN|WARN/i.test(line)
        ? 'warn'
        : 'info',
    message: line,
  }));

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">日志与内核能力矩阵</h1>
          <p className="text-xs text-[var(--text-secondary)]">
            实时探测 Android GKI 原厂内核与 Tier 2 定制内核特性，并查阅 stratad / 容器运行日志
          </p>
        </div>
        <Button size="sm" variant="secondary" disabled={loading} onClick={() => void loadLogs()}>
          <RefreshCw className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
          刷新日志
        </Button>
      </div>

      {/* Kernel Capability Matrix */}
      <Card className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Cpu className="size-4 text-[var(--accent)]" />
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              内核能力矩阵 (`daemon.capabilities`)
            </span>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="neutral">{capabilities?.kernelRelease ?? '等待探测…'}</Badge>
            <Badge variant={capabilities?.dockerReady ? 'success' : 'accent'}>
              {capabilities?.dockerReady ? 'Tier 2 完整内核' : '原厂 GKI 兼容模式'}
            </Badge>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {CAP_MATRIX.map((cap) => {
            const supported = capabilities ? capabilities[cap.key] : false;
            return (
              <div
                key={cap.key}
                className="flex items-center justify-between gap-2 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-xs font-medium text-[var(--text-primary)]">
                      {cap.label}
                    </span>
                    <Badge variant={cap.tier === 'stock-gki' ? 'neutral' : 'accent'}>
                      {cap.tier === 'stock-gki' ? 'GKI' : 'Tier 2'}
                    </Badge>
                  </div>
                  <div className="truncate text-[11px] text-[var(--text-secondary)]">
                    {cap.hint}
                  </div>
                </div>
                {supported ? (
                  <CheckCircle2 className="size-4 shrink-0 text-[var(--success)]" />
                ) : (
                  <XCircle className="size-4 shrink-0 text-[var(--text-secondary)] opacity-60" />
                )}
              </div>
            );
          })}
        </div>
      </Card>

      {/* Log Source Switcher + Console */}
      <div className="flex items-center justify-between gap-2">
        <SegmentedControl
          size="sm"
          value={logSource}
          onValueChange={setLogSource}
          options={[
            { value: 'daemon', label: 'stratad 守护进程' },
            ...containers.map((c) => ({ value: c.id, label: `容器 ${c.id}` })),
          ]}
        />
      </div>

      <LogConsole
        title={logSource === 'daemon' ? 'stratad.log' : `${logSource}.log`}
        entries={entries}
        searchable
        filterable
        maxHeightClassName="max-h-[420px]"
        emptyMessage="暂无日志记录"
      />
    </div>
  );
}
