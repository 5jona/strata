import type {
  InstalledToolItem,
  ToolGroupKind,
  ToolInstallSource,
  ToolManagerKind,
  ToolsConfigResult,
  ToolsConfigSetParams,
  ToolsDoctorResult,
  ToolsStatusResult,
} from '../types/protocol.ts';

const MB = 1024 * 1024;

export const TUNA_MIRRORS = {
  aptMirrorUrl: 'https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports',
  nodeMirrorUrl: 'https://mirrors.tuna.tsinghua.edu.cn/nodejs-release/',
  pypiIndexUrl: 'https://pypi.tuna.tsinghua.edu.cn/simple',
  npmRegistryUrl: 'https://registry.npmmirror.com',
  rustupDistServer: 'https://mirrors.tuna.tsinghua.edu.cn/rustup',
  goproxy: 'https://goproxy.cn,direct',
} as const;

const CONFIG_STR_FIELDS = [
  'aptMirrorUrl',
  'nodeMirrorUrl',
  'pypiIndexUrl',
  'npmRegistryUrl',
  'rustupDistServer',
  'goproxy',
  'httpProxy',
  'httpsProxy',
  'noProxy',
  'minimumReleaseAge',
] as const;

export function applyMockMirrorPreset(
  cfg: ToolsConfigResult,
  preset: 'tuna' | 'official' | 'custom',
) {
  cfg.mirrorPreset = preset;
  if (preset === 'tuna') Object.assign(cfg, TUNA_MIRRORS);
  else if (preset === 'official') {
    for (const k of Object.keys(TUNA_MIRRORS) as (keyof typeof TUNA_MIRRORS)[]) {
      cfg[k] = undefined;
    }
  }
}

export function applyMockToolsConfigSet(state: MockToolsState, params: ToolsConfigSetParams) {
  const cfg = state.config;
  if (params.mirrorPreset) {
    applyMockMirrorPreset(cfg, params.mirrorPreset);
    state.status.mirrorPreset = params.mirrorPreset;
  }
  for (const k of CONFIG_STR_FIELDS) {
    const val = params[k];
    if (val === undefined) continue;
    if (!val.trim() && (params.mirrorPreset === 'tuna' || params.mirrorPreset === 'official'))
      continue;
    cfg[k] = val.trim() || undefined;
  }
  if (params.urlReplacements !== undefined) cfg.urlReplacements = params.urlReplacements;
  if (params.jobs !== undefined) cfg.jobs = params.jobs;
  const targetPath = params.configPath || cfg.configPath || '/root/.config/mise/config.toml';
  cfg.configPath = targetPath;
  if (!cfg.configPaths.includes(targetPath)) cfg.configPaths = [...cfg.configPaths, targetPath];
  if (params.miseConfigToml !== undefined) {
    cfg.miseConfigToml = params.miseConfigToml;
    state.configFiles[targetPath] = params.miseConfigToml;
  }
  if (params.githubToken !== undefined) {
    const tok = params.githubToken.trim();
    cfg.hasGithubToken = Boolean(tok);
    cfg.githubTokenMasked = tok
      ? tok.length <= 8
        ? '****'
        : `${tok.slice(0, 4)}...${tok.slice(-4)}`
      : undefined;
    state.status.hasGithubToken = cfg.hasGithubToken;
  }
  return cfg;
}

const toolItem = (
  id: string,
  name: string,
  manager: ToolManagerKind,
  group: ToolGroupKind,
  version: string,
  installPath: string,
  source: ToolInstallSource,
  extra?: Partial<InstalledToolItem>,
): InstalledToolItem => ({
  id,
  name,
  manager,
  group,
  version,
  installPath,
  source,
  outdated: false,
  active: true,
  installed: true,
  broken: false,
  ...extra,
});

const mgr = (version: string, binPath: string, source: ToolInstallSource) => ({
  installed: true,
  version,
  binPath,
  source,
});

const doc = (id: string, category: string, summary: string) => ({
  id,
  category,
  status: 'ok' as const,
  summary,
});

export interface MockToolsState {
  status: ToolsStatusResult;
  items: InstalledToolItem[];
  config: ToolsConfigResult;
  configFiles: Record<string, string>;
  doctor: ToolsDoctorResult;
}

export function createInitialToolsState(container: string, running: boolean): MockToolsState {
  const userToml = [
    '[settings]',
    'jobs = 4',
    'minimum_release_age = "24h"',
    '',
    '[tools]',
    'node = "22"',
    'python = "3.12"',
  ].join('\n');
  const strataToml = '# /etc/strata/mise/config.toml\n[settings]\njobs = 4\n\n[tools]\n';
  const configPaths = ['/root/.config/mise/config.toml', '/etc/strata/mise/config.toml'];

  return {
    status: {
      container,
      layoutSchemaVersion: 2,
      layoutCompliant: running,
      workspaceDir: '/workspace',
      miseConfigPath: configPaths[0] ?? '/root/.config/mise/config.toml',
      miseDataDir: '/root/.local/share/mise',
      miseConfigPaths: configPaths,
      legacyStrataMiseDetected: true,
      systemPkgManager: 'apt',
      mise: mgr('2026.11.0 linux-arm64', '/root/.local/bin/mise', 'user-installed'),
      uv: mgr('uv 0.9.12', '/root/.local/bin/uv', 'user-installed'),
      apt: mgr('apt 2.8.3 (arm64)', '/usr/bin/apt-get', 'system'),
      hostExec: mgr('1.0', '/usr/local/bin/host-exec', 'strata-managed'),
      strataDebug: mgr('0.2.0', '/usr/local/bin/strata-debug', 'strata-managed'),
      installedToolCount: running ? 7 : 0,
      outdatedToolCount: 1,
      mirrorPreset: 'tuna',
      hasGithubToken: false,
      cacheBytes: 142 * MB,
      installsBytes: 386 * MB,
    },
    items: [
      toolItem(
        'system:host-exec',
        'host-exec',
        'system',
        'core-bridge',
        '1.0',
        '/usr/local/bin/host-exec',
        'strata-managed',
      ),
      toolItem(
        'apt:dropbear-bin',
        'dropbear-bin',
        'apt',
        'core-bridge',
        '2022.83-4',
        '/usr/sbin/dropbear',
        'system',
      ),
      toolItem(
        'mise:python@3.12.9',
        'python',
        'mise',
        'mise-runtimes',
        '3.12.9',
        '/root/.local/share/mise/installs/python/3.12.9',
        'user-installed',
        {
          requestedVersion: '3.12',
          backend: 'core:python',
          sourcePath: '/root/.config/mise/config.toml',
        },
      ),
      toolItem(
        'mise:python@3.11.11',
        'python',
        'mise',
        'mise-runtimes',
        '3.11.11',
        '/root/.local/share/mise/installs/python/3.11.11',
        'user-installed',
        {
          backend: 'core:python',
          active: false,
        },
      ),
      toolItem(
        'mise:node@22.12.0',
        'node',
        'mise',
        'mise-runtimes',
        '22.12.0',
        '/root/.local/share/mise/installs/node/22.12.0',
        'user-installed',
        {
          requestedVersion: '22',
          latestVersion: '22.14.0',
          outdated: true,
          backend: 'core:node',
          sourcePath: '/root/.config/mise/config.toml',
        },
      ),
      toolItem(
        'uv:ruff',
        'ruff',
        'uv',
        'python-uv',
        '0.9.4',
        '/root/.local/share/uv/tools/ruff',
        'user-installed',
      ),
      toolItem(
        'manual:custom-cli',
        'custom-cli',
        'manual',
        'discovered',
        'binary',
        '/root/.local/bin/custom-cli',
        'user-installed',
      ),
    ],
    config: {
      container,
      configPath: configPaths[0] ?? '/root/.config/mise/config.toml',
      configPaths,
      mirrorPreset: 'tuna',
      ...TUNA_MIRRORS,
      noProxy: 'localhost,127.0.0.1',
      urlReplacements: {},
      jobs: 4,
      minimumReleaseAge: '24h',
      hasGithubToken: false,
      miseConfigToml: userToml,
    },
    configFiles: {
      '/root/.config/mise/config.toml': userToml,
      '/etc/strata/mise/config.toml': strataToml,
    },
    doctor: {
      container,
      ok: true,
      items: [
        doc('canonical-layout', 'layout', '标准目录布局 schema v2 已就绪'),
        doc(
          'login-shell-env',
          'environment',
          '登录 Shell 已暴露 mise (/root/.local/bin/mise) 与 shims PATH',
        ),
        doc('apt-sandbox-root', 'apt', 'APT::Sandbox::User "root" 已配置'),
        doc('dropbear-env-pass', 'ssh', 'Dropbear 以仅公钥模式运行并保留容器环境变量 (-e)'),
      ],
      miseDoctorJson: JSON.stringify(
        { version: '2026.11.0', os: 'linux', arch: 'arm64', problems: [] },
        null,
        2,
      ),
    },
  };
}
