import type {
  ContainerPhase,
  ContainerStatusSnapshot,
  MaintenanceDonePayload,
  ServiceStatus,
} from '../types.ts';

/** Execution mode supported by the headless task runner input. */
export type TaskRunnerMode = 'shell' | 'python';

/** Built-in one-click diagnostic command preset for the Tasks tab. */
export interface QuickTaskPreset {
  id: string;
  label: string;
  mode: TaskRunnerMode;
  script: string;
}

/** Toast presentation descriptor derived from a maintenance completion event. */
export interface MaintenanceToastDescriptor {
  title: string;
  tone: 'success' | 'danger';
  description?: string;
}

/** Display metadata mapping each container lifecycle phase to a badge variant and label. */
export const PHASE_META: Record<
  ContainerPhase,
  { label: string; variant: 'success' | 'neutral' | 'warning' | 'danger' | 'accent' }
> = {
  running: { label: '运行中', variant: 'success' },
  stopped: { label: '已停止', variant: 'neutral' },
  starting: { label: '启动中', variant: 'accent' },
  stopping: { label: '停止中', variant: 'warning' },
  failed: { label: '异常', variant: 'danger' },
};

/** Display metadata mapping each managed service status to a badge variant and label. */
export const SERVICE_STATUS_META: Record<
  ServiceStatus,
  { label: string; variant: 'success' | 'neutral' | 'warning' | 'danger' }
> = {
  running: { label: '运行中', variant: 'success' },
  stopped: { label: '已停止', variant: 'neutral' },
  crashed: { label: '已崩溃', variant: 'danger' },
  'circuit-open': { label: '熔断', variant: 'warning' },
};

const MAINTENANCE_OP_LABELS: Readonly<Record<string, string>> = {
  backup: '后台备份',
  compact: '空间回收',
  fsck: '磁盘检查',
  dns: 'DNS 同步',
};

/** Built-in quick task presets exposed in the Tasks tab. */
export const QUICK_TASK_PRESETS: readonly QuickTaskPreset[] = [
  {
    id: 'sysinfo',
    label: '环境概览',
    mode: 'shell',
    script: 'uname -a && uptime && df -h /',
  },
  {
    id: 'pyver',
    label: 'Python 自检',
    mode: 'python',
    script: 'import platform, sys; print(sys.version.splitlines()[0], platform.machine())',
  },
  {
    id: 'host-prop',
    label: '宿主机桥接',
    mode: 'shell',
    script: 'host-exec getprop ro.build.version.release && host-exec id',
  },
  {
    id: 'net-check',
    label: '网络与 DNS',
    mode: 'shell',
    script: 'cat /etc/resolv.conf && ip -4 -br addr',
  },
];

/** Formats a byte count into a human-readable IEC unit string. */
export function formatBytes(bytes: number): string {
  const { value, unit } = splitBytes(bytes);
  return `${value} ${unit}`;
}

/** Splits a byte count into a formatted numeric value and IEC unit suffix. */
export function splitBytes(bytes: number): { value: string; unit: string } {
  if (!Number.isFinite(bytes) || bytes < 0) return { value: '0', unit: 'B' };
  const kib = 1024;
  const mib = kib * 1024;
  const gib = mib * 1024;
  if (bytes >= gib) return { value: (bytes / gib).toFixed(2), unit: 'GiB' };
  if (bytes >= mib) return { value: (bytes / mib).toFixed(1), unit: 'MiB' };
  if (bytes >= kib) return { value: (bytes / kib).toFixed(1), unit: 'KiB' };
  return { value: String(Math.round(bytes)), unit: 'B' };
}

/** Formats container uptime in milliseconds as a concise duration string. */
export function formatUptime(uptimeMs: number): string {
  if (!Number.isFinite(uptimeMs) || uptimeMs <= 0) return '未运行';
  const totalSecs = Math.floor(uptimeMs / 1000);
  const hours = Math.floor(totalSecs / 3600);
  const mins = Math.floor((totalSecs % 3600) / 60);
  const secs = totalSecs % 60;
  if (hours > 0) return `${hours}h ${mins}m`;
  if (mins > 0) return `${mins}m ${secs}s`;
  return `${secs}s`;
}

/** Builds a ready-to-copy SSH CLI command for connecting to a container. */
export function buildSshCommand(snapshot: ContainerStatusSnapshot): string {
  const host = snapshot.primaryIpv4 || '127.0.0.1';
  return `ssh -p ${snapshot.sshPort} root@${host}`;
}

/** Wraps a user command string into an argv array for shell or Python execution. */
export function buildTaskCommand(mode: TaskRunnerMode, input: string): string[] {
  const trimmed = input.trim();
  if (!trimmed) return [];
  if (mode === 'python') {
    return ['/usr/bin/python3', '-c', trimmed];
  }
  return ['/bin/sh', '-c', trimmed];
}

/** Formats a Unix timestamp in milliseconds as `HH:MM:SS` in local time. */
export function formatClockTime(tsMs: number): string {
  const date = new Date(tsMs);
  const hh = String(date.getHours()).padStart(2, '0');
  const mm = String(date.getMinutes()).padStart(2, '0');
  const ss = String(date.getSeconds()).padStart(2, '0');
  return `${hh}:${mm}:${ss}`;
}

/** Extracts the key algorithm and a redacted display label from an authorized_keys line. */
export function summarizeSshKey(keyLine: string): { algo: string; label: string } {
  const parts = keyLine.trim().split(/\s+/);
  const algo = parts[0] ?? 'ssh-key';
  const comment = parts.slice(2).join(' ');
  const body = parts[1] ?? '';
  const fingerprintHint =
    body.length > 18 ? `${body.slice(0, 10)}…${body.slice(-8)}` : body || '无内容';
  return {
    algo,
    label: comment ? `${comment} (${fingerprintHint})` : fingerprintHint,
  };
}

/** Parses and validates a TCP port string within the `[1, 65535]` range. */
export function parseSshPort(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 65535) return null;
  return parsed;
}

/** Resolves a maintenance operation identifier to its localized display label. */
export function formatMaintenanceOpLabel(op: string): string {
  return MAINTENANCE_OP_LABELS[op] ?? `维护任务 (${op})`;
}

/** Formats toast title, tone, and description from a `container.maintenanceDone` payload. */
export function formatMaintenanceToast(
  payload: MaintenanceDonePayload,
): MaintenanceToastDescriptor {
  const label = formatMaintenanceOpLabel(payload.op);
  if (payload.ok) {
    return {
      title: `${label}完成`,
      tone: 'success',
      description: payload.summary,
    };
  }
  return {
    title: `${label}失败`,
    tone: 'danger',
    description: payload.error ?? payload.summary,
  };
}
