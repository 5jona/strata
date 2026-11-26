/** Actionable guidance shown when SideVeil rejects clipboard or native-hud capabilities. */
export const SIDEVEIL_PERMISSION_GUIDE =
  '权限尚未开启，请前往 SideVeil 控制中心 → 已安装插件 → Strata 容器 → 审查权限，启用剪贴板或悬浮状态条能力';

/** Returns true when a host/bridge error indicates missing or denied SideVeil plugin permissions. */
export function isSideveilPermissionError(err: unknown): boolean {
  if (err == null) return false;
  const parts: string[] = [];
  if (err instanceof Error) {
    parts.push(err.message);
  }
  if (typeof err === 'object') {
    const record = err as Record<string, unknown>;
    if (record.code !== undefined) parts.push(String(record.code));
    if (record.message !== undefined) parts.push(String(record.message));
  }
  parts.push(String(err));
  const text = parts.join(' ').toLowerCase();
  return (
    text.includes('-32003') ||
    text.includes('permission') ||
    text.includes('denied') ||
    text.includes('unauthorized') ||
    text.includes('未授权') ||
    text.includes('未开启')
  );
}
