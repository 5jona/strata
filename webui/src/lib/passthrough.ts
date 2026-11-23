import { type BindMount, L5_DIAG_MOUNTS } from '../types.ts';

export interface CommonCharDevOption {
  name: string;
  label: string;
  description: string;
}

export const COMMON_DEBUG_CHAR_DEVS: readonly CommonCharDevOption[] = [
  { name: 'uinput', label: 'uinput', description: '虚拟输入设备' },
  { name: 'net/tun', label: 'net/tun', description: '虚拟网络设备' },
];

const COMMON_CHAR_DEV_SET = new Set(COMMON_DEBUG_CHAR_DEVS.map((item) => item.name));

function isSameMountPath(a: BindMount, b: BindMount): boolean {
  return a.hostPath === b.hostPath && a.containerPath === b.containerPath;
}

export function isL5DiagMount(mount: BindMount): boolean {
  return L5_DIAG_MOUNTS.some((l5) => l5.containerPath === mount.containerPath);
}

export function hasL5DiagMounts(mounts: readonly BindMount[]): boolean {
  return L5_DIAG_MOUNTS.every((l5) =>
    mounts.some((m) => isSameMountPath(l5, m) && m.readOnly === l5.readOnly),
  );
}

/**
 * Merges or removes the canonical read-only L5 diagnostic mounts without
 * modifying any other user-configured bind mounts.
 */
export function toggleL5DiagMounts(mounts: readonly BindMount[], enabled: boolean): BindMount[] {
  const custom = mounts.filter((m) => !isL5DiagMount(m)).map((m) => ({ ...m }));
  if (!enabled) {
    return custom;
  }
  return [...custom, ...L5_DIAG_MOUNTS.map((m) => ({ ...m }))];
}

export function getCustomMounts(mounts: readonly BindMount[]): BindMount[] {
  return mounts.filter((m) => !isL5DiagMount(m));
}

/**
 * Toggles a character device entry while preserving order and any unknown entries.
 */
export function toggleCharDev(
  devs: readonly string[],
  devName: string,
  enabled: boolean,
): string[] {
  const normalized = devs.filter((item) => item.trim().length > 0);
  if (enabled) {
    if (normalized.includes(devName)) {
      return [...normalized];
    }
    return [...normalized, devName];
  }
  return normalized.filter((item) => item !== devName);
}

export function getCustomCharDevs(devs: readonly string[]): string[] {
  return devs.filter((item) => !COMMON_CHAR_DEV_SET.has(item));
}
