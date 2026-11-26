import type { ContainerStatusSnapshot } from '../../types/protocol.ts';

export type InstallSourceMode = 'online' | 'url' | 'local' | 'clone';

const CONTAINER_ID_RE = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const SHA256_RE = /^[a-fA-F0-9]{64}$/;

export function validateContainerIdInput(
  raw: string,
  containers: ContainerStatusSnapshot[],
): string | null {
  const id = raw.trim();
  if (!id) return '请输入容器名称 (ID)';
  if (id.length < 2 || id.length > 32) return '容器名称长度需为 2～32 个字符';
  if (!CONTAINER_ID_RE.test(id)) {
    return '仅支持小写字母、数字和单个连字符 (-)，且必须以小写字母开头、字母或数字结尾';
  }
  if (containers.some((c) => c.id === id)) {
    return `容器「${id}」已存在，请更换名称`;
  }
  return null;
}

export function validateSshPortInput(
  enableSsh: boolean,
  rawPort: string,
  containers: ContainerStatusSnapshot[],
): string | null {
  if (!enableSsh) return null;
  const num = Number(rawPort.trim());
  if (!Number.isInteger(num) || num < 1024 || num > 65535) {
    return 'SSH 端口需为 1024～65535 之间的整数';
  }
  const conflict = containers.find((c) => c.sshEnabled && c.sshPort === num);
  if (conflict) {
    return `端口 ${num} 已被容器「${conflict.id}」占用`;
  }
  return null;
}

export function validateHttpsUrlInput(raw: string, label: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return `请输入${label}`;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'https:') return `${label}必须使用 https:// 协议`;
    return null;
  } catch {
    return `${label}格式无效`;
  }
}

export function validateSha256Input(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (!SHA256_RE.test(trimmed)) return 'SHA256 校验值应为 64 位十六进制字符';
  return null;
}

export function suggestContainerId(baseRaw: string, containers: ContainerStatusSnapshot[]): string {
  const clean = baseRaw
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^[^a-z]+/, '')
    .replace(/-+$/, '')
    .slice(0, 26);
  const base = clean.length >= 2 ? clean : 'linux-dev';
  const existing = new Set(containers.map((c) => c.id));
  if (!existing.has(base)) return base;
  for (let i = 2; i < 100; i += 1) {
    const candidate = `${base.slice(0, 28)}-${i}`;
    if (!existing.has(candidate)) return candidate;
  }
  return `${base.slice(0, 24)}-${Date.now().toString(36).slice(-4)}`;
}

export function suggestAvailableSshPort(containers: ContainerStatusSnapshot[]): number {
  const used = new Set(containers.filter((c) => c.sshEnabled).map((c) => c.sshPort));
  let candidate = 2222;
  while (used.has(candidate) && candidate < 65535) {
    candidate += 1;
  }
  return candidate;
}

export function parsePublicKeysInput(raw: string): string[] {
  return raw
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

export function isRawDiskImageFilename(filename: string): boolean {
  const lower = filename.toLowerCase();
  return lower.endsWith('.img') || lower.endsWith('.img.gz') || lower.endsWith('.ext4');
}

export const JOB_STAGE_LABELS: Record<string, string> = {
  resolving: '解析镜像源',
  downloading: '下载 RootFS',
  uploading: '流式上传文件',
  importing: '接收镜像流',
  formatting_ext4: '创建稀疏镜像',
  writing_image: '写入磁盘镜像',
  extracting: '解压 RootFS',
  provisioning: '配置容器环境',
  completed: '已完成',
  done: '已完成',
  failed: '失败',
  cancelled: '已取消',
};
