import { type BindMount, L5_DIAG_PATHS } from '../types/protocol.ts';

export interface CommonCharDevOption {
  name: string;
  label: string;
  description: string;
}

/** Canonical read-only L5 kernel diagnostic bind mounts. */
export const L5_DIAG_MOUNTS: readonly BindMount[] = [
  { hostPath: L5_DIAG_PATHS.PSTORE, containerPath: L5_DIAG_PATHS.PSTORE, readOnly: true },
  { hostPath: L5_DIAG_PATHS.TRACING, containerPath: L5_DIAG_PATHS.TRACING, readOnly: true },
  { hostPath: L5_DIAG_PATHS.BPF, containerPath: L5_DIAG_PATHS.BPF, readOnly: true },
];

/** Frequently used Android debugging character devices exposed as quick toggles. */
export const COMMON_DEBUG_CHAR_DEVS: readonly CommonCharDevOption[] = [
  { name: 'uinput', label: 'uinput', description: '虚拟输入设备' },
  { name: 'net/tun', label: 'net/tun', description: '虚拟网络设备' },
];

const COMMON_CHAR_DEV_SET = new Set(COMMON_DEBUG_CHAR_DEVS.map((item) => item.name));

function isSameMountPath(a: BindMount, b: BindMount): boolean {
  return a.hostPath === b.hostPath && a.containerPath === b.containerPath;
}

/** Returns true if a bind mount targets any canonical L5 kernel diagnostic path. */
export function isL5DiagMount(mount: BindMount): boolean {
  return L5_DIAG_MOUNTS.some((l5) => l5.containerPath === mount.containerPath);
}

/** Returns true when every canonical read-only L5 diagnostic mount is configured. */
export function hasL5DiagMounts(mounts: readonly BindMount[]): boolean {
  return L5_DIAG_MOUNTS.every((l5) =>
    mounts.some((m) => isSameMountPath(l5, m) && m.readOnly === l5.readOnly),
  );
}

/** Merges or removes canonical read-only L5 diagnostic mounts without touching custom mounts. */
export function toggleL5DiagMounts(mounts: readonly BindMount[], enabled: boolean): BindMount[] {
  const custom = mounts.filter((m) => !isL5DiagMount(m)).map((m) => ({ ...m }));
  if (!enabled) return custom;
  return [...custom, ...L5_DIAG_MOUNTS.map((m) => ({ ...m }))];
}

/** Returns only user-defined bind mounts (excluding canonical L5 diagnostic mounts). */
export function getCustomMounts(mounts: readonly BindMount[]): BindMount[] {
  return mounts.filter((m) => !isL5DiagMount(m));
}

/** Toggles a character device entry while preserving order and unknown entries. */
export function toggleCharDev(
  devs: readonly string[],
  devName: string,
  enabled: boolean,
): string[] {
  const normalized = devs.filter((item) => item.trim().length > 0);
  if (enabled) {
    return normalized.includes(devName) ? [...normalized] : [...normalized, devName];
  }
  return normalized.filter((item) => item !== devName);
}

/** Returns character device entries not covered by built-in quick toggles. */
export function getCustomCharDevs(devs: readonly string[]): string[] {
  return devs.filter((item) => !COMMON_CHAR_DEV_SET.has(item));
}
