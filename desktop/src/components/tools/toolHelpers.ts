import type {
  InstalledToolItem,
  ToolGroupKind,
  ToolInstallSource,
  ToolManagerKind,
  ToolMirrorPreset,
} from '../../types/protocol.ts';

export const TOOL_GROUP_LABELS: Record<'all' | ToolGroupKind, string> = {
  all: '全部',
  'mise-runtimes': '编程语言运行时',
  'cli-dev': '开发 CLI 工具',
  shell: '终端增强 (Shell)',
  'python-uv': 'Python / uv',
  'debug-l3-l5': '系统诊断工具',
  discovered: '本机自装发现',
  'core-bridge': '基础桥接',
};

export const TOOL_GROUP_ORDER: Array<'all' | ToolGroupKind> = [
  'all',
  'mise-runtimes',
  'shell',
  'cli-dev',
  'python-uv',
  'debug-l3-l5',
  'discovered',
  'core-bridge',
];

export function toolGroupLabel(group: ToolGroupKind): string {
  return TOOL_GROUP_LABELS[group] ?? group;
}

export function toolSourceMeta(source?: ToolInstallSource): {
  label: string;
  variant: 'accent' | 'neutral' | 'success';
} {
  switch (source) {
    case 'user-installed':
      return { label: '用户自装', variant: 'accent' };
    case 'strata-managed':
      return { label: 'Strata 托管', variant: 'success' };
    default:
      return { label: '系统自带', variant: 'neutral' };
  }
}

export function managerLabel(manager: ToolManagerKind): string {
  switch (manager) {
    case 'system':
      return '系统桥接';
    case 'manual':
      return '本机二进制';
    default:
      return manager;
  }
}

const PROTECTED_CORE_PACKAGES = new Set([
  'dropbear',
  'dropbear-bin',
  'openssh-sftp-server',
  'ca-certificates',
  'libc6',
  'bash',
  'host-exec',
  'strata-debug',
]);

export function isProtectedTool(item: InstalledToolItem): boolean {
  return (
    item.manager === 'system' ||
    item.manager === 'manual' ||
    item.group === 'core-bridge' ||
    PROTECTED_CORE_PACKAGES.has(item.name.toLowerCase())
  );
}

export interface GroupedToolEntry {
  key: string;
  name: string;
  manager: ToolManagerKind;
  group: ToolGroupKind;
  source?: ToolInstallSource;
  activeItem: InstalledToolItem;
  versions: InstalledToolItem[];
}

export function groupInstalledTools(items: InstalledToolItem[]): GroupedToolEntry[] {
  const map = new Map<string, InstalledToolItem[]>();
  for (const item of items) {
    const key = `${item.manager}:${item.name}`;
    const list = map.get(key);
    if (list) list.push(item);
    else map.set(key, [item]);
  }
  const groups: GroupedToolEntry[] = [];
  for (const [key, versions] of map.entries()) {
    const sorted = [...versions].sort((a, b) => Number(b.active) - Number(a.active));
    const activeItem = sorted[0];
    if (!activeItem) continue;
    groups.push({
      key,
      name: activeItem.name,
      manager: activeItem.manager,
      group: activeItem.group,
      ...(activeItem.source ? { source: activeItem.source } : {}),
      activeItem,
      versions: sorted,
    });
  }
  return groups;
}

export const DOCTOR_ITEM_TITLES: Record<string, { title: string; category: string }> = {
  'canonical-layout': { title: '容器标准目录与环境布局', category: '目录布局' },
  'login-shell-env': { title: '登录 Shell 环境变量与 PATH', category: 'Shell 环境' },
  'apt-sandbox-root': { title: 'APT 包管理沙盒兼容配置', category: '包管理' },
  'dropbear-env-pass': { title: 'Dropbear SSH 环境变量透传', category: 'SSH 服务' },
  'container-mounted': { title: '容器根文件系统挂载状态', category: '运行状态' },
  'rootfs-open': { title: '容器根文件系统访问检查', category: '运行状态' },
  'mise-doctor': { title: 'mise 运行时健康自检', category: 'mise 工具链' },
};

export interface MirrorPresetUrls {
  aptMirrorUrl: string;
  nodeMirrorUrl: string;
  pypiIndexUrl: string;
  npmRegistryUrl: string;
  rustupDistServer: string;
  goproxy: string;
}

export const TUNA_PRESET_URLS: MirrorPresetUrls = {
  aptMirrorUrl: 'https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports',
  nodeMirrorUrl: 'https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/',
  pypiIndexUrl: 'https://pypi.tuna.tsinghua.edu.cn/simple',
  npmRegistryUrl: 'https://registry.npmmirror.com',
  rustupDistServer: 'https://mirrors.tuna.tsinghua.edu.cn/rustup',
  goproxy: 'https://goproxy.cn,direct',
};

export const OFFICIAL_PRESET_URLS: MirrorPresetUrls = {
  aptMirrorUrl: '',
  nodeMirrorUrl: '',
  pypiIndexUrl: '',
  npmRegistryUrl: '',
  rustupDistServer: '',
  goproxy: '',
};

export function presetMirrorDefaults(preset: ToolMirrorPreset): MirrorPresetUrls | null {
  if (preset === 'tuna') return TUNA_PRESET_URLS;
  if (preset === 'official') return OFFICIAL_PRESET_URLS;
  return null;
}

export const SHELL_PRESET_META: Record<
  string,
  { tag: string; tagVariant: 'success' | 'accent' | 'warning'; note: string }
> = {
  'shell-zsh': {
    tag: '推荐 · 兼容 POSIX',
    tagVariant: 'success',
    note: '自动补全、命令建议与语法高亮即装即用，完全兼容现有 Shell 脚本。',
  },
  'shell-fish': {
    tag: '开箱智能补全',
    tagVariant: 'accent',
    note: '无需复杂配置文件即可获得开箱即用的历史提示与交互补全体验。',
  },
  'shell-nushell': {
    tag: '结构化进阶',
    tagVariant: 'warning',
    note: '面向结构化数据管道的现代 Shell（非 POSIX 语法，适合进阶用户）。',
  },
};
