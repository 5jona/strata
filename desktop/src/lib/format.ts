import type { SshTarget } from '../types/device.ts';
import type { ContainerPhase, ContainerServiceSpec, ServiceStatus } from '../types/protocol.ts';

/** Formats a byte count into a concise IEC unit string (`GiB`, `MiB`, `KiB`, `B`). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let idx = 0;
  let value = bytes;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value >= 10 || idx === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[idx]}`;
}

/** Formats milliseconds into a compact human-readable Chinese duration. */
export function formatUptime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '未运行';
  const totalSecs = Math.floor(ms / 1000);
  const days = Math.floor(totalSecs / 86400);
  const hours = Math.floor((totalSecs % 86400) / 3600);
  const mins = Math.floor((totalSecs % 3600) / 60);
  const secs = totalSecs % 60;
  if (days > 0) return `${days}天 ${hours}小时`;
  if (hours > 0) return `${hours}小时 ${mins}分`;
  if (mins > 0) return `${mins}分 ${secs}秒`;
  return `${secs}秒`;
}

/** Returns localized badge label and visual tone for a container lifecycle phase. */
export function phaseBadgeMeta(phase: ContainerPhase): {
  label: string;
  variant: 'success' | 'warning' | 'danger' | 'neutral' | 'accent';
} {
  switch (phase) {
    case 'running':
      return { label: '运行中', variant: 'success' };
    case 'starting':
      return { label: '启动中', variant: 'accent' };
    case 'stopping':
      return { label: '停止中', variant: 'warning' };
    case 'failed':
      return { label: '异常', variant: 'danger' };
    case 'stopped':
      return { label: '已停止', variant: 'neutral' };
  }
}

/** Returns localized badge label and visual tone for a managed service status. */
export function serviceStatusMeta(status: ServiceStatus): {
  label: string;
  variant: 'success' | 'warning' | 'danger' | 'neutral';
} {
  switch (status) {
    case 'running':
      return { label: '运行中', variant: 'success' };
    case 'crashed':
      return { label: '已崩溃', variant: 'danger' };
    case 'circuit-open':
      return { label: '已熔断（连续失败过多）', variant: 'warning' };
    case 'stopped':
      return { label: '已停止', variant: 'neutral' };
  }
}

const MAINTENANCE_OP_LABELS: Readonly<Record<string, string>> = {
  backup: '后台备份',
  compact: '回收磁盘空间',
  fsck: '磁盘修复检查',
  dns: '同步 DNS',
};

/** Maps a maintenance operation id to a localized Chinese label. */
export function formatMaintenanceOpLabel(op: string): string {
  return MAINTENANCE_OP_LABELS[op] ?? op;
}

/** Parses and validates a TCP port string within the inclusive `[min, max]` range. */
export function parsePortInRange(raw: string, min = 1024, max = 65535): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) return null;
  return parsed;
}

/** Builds a copyable SSH CLI command for a container. */
export function formatSshCommand(target: SshTarget): string {
  return `ssh -p ${target.port} ${target.user}@${target.host}`;
}

/** Builds a `vscode://vscode-remote/ssh-remote+...` deep link URI. */
export function formatVsCodeRemoteUri(target: SshTarget): string {
  const path = target.remotePath?.startsWith('/') ? target.remotePath : '/root';
  return `vscode://vscode-remote/ssh-remote+${target.user}@${target.host}:${target.port}${path}`;
}

/** Builds a `~/.ssh/config` block with ControlMaster multiplexing. */
export function formatSshConfigSnippet(target: SshTarget): string {
  const alias = target.alias || 'strata-main';
  return [
    `Host ${alias}`,
    `    HostName ${target.host}`,
    `    Port ${target.port}`,
    `    User ${target.user}`,
    '    ServerAliveInterval 30',
    '    ControlMaster auto',
    '    ControlPath ~/.ssh/cm-%r@%h:%p',
    '    ControlPersist 600',
  ].join('\n');
}

/** Parses `KEY=VALUE` lines into an environment record. */
export function parseEnvLines(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    const val = line.slice(eq + 1).trim();
    if (key) env[key] = val;
  }
  return env;
}

/** Serializes an environment record into `KEY=VALUE` lines. */
export function formatEnvLines(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join('\n');
}

/** Splits a command string into argv tokens respecting simple single/double quotes. */
export function parseCommandArgv(input: string): string[] {
  const tokens: string[] = [];
  let current = '';
  let quote: '"' | "'" | null = null;
  for (const ch of input.trim()) {
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (current) {
        tokens.push(current);
        current = '';
      }
    } else {
      current += ch;
    }
  }
  if (current) tokens.push(current);
  return tokens;
}

/** Validates a managed service spec before submitting `service.upsert`. */
export function validateServiceSpec(spec: ContainerServiceSpec): string | null {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(spec.id)) {
    return '服务 ID 须为 1-64 位字母、数字、连字符或下划线';
  }
  if (!spec.name.trim()) {
    return '服务名称不能为空';
  }
  if (spec.command.length === 0) {
    return '启动命令不能为空';
  }
  if (spec.maxRestarts < 0 || spec.maxRestarts > 100) {
    return '最大重启次数须在 0-100 之间';
  }
  if (spec.backoffMs < 100 || spec.backoffMs > 300_000) {
    return '退避延迟须在 100ms-300000ms 之间';
  }
  return null;
}

/** Summarizes an OpenSSH public key line into algorithm + label. */
export function summarizeSshKey(line: string): { algo: string; label: string } {
  const parts = line.trim().split(/\s+/);
  const algo = parts[0] ?? 'ssh-key';
  const comment = parts.slice(2).join(' ');
  const body = parts[1] ?? '';
  const shortBody = body.length > 18 ? `${body.slice(0, 10)}…${body.slice(-6)}` : body;
  return {
    algo: algo.replace(/^ssh-/, '').toUpperCase(),
    label: comment || shortBody || line,
  };
}
