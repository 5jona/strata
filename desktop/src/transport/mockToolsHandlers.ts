import type {
  InstalledToolItem,
  ToolGroupKind,
  ToolManagerKind,
  ToolsBootstrapParams,
  ToolsCacheCleanParams,
  ToolsCatalogParams,
  ToolsConfigSetParams,
  ToolsInstallParams,
  ToolsListParams,
  ToolsUninstallParams,
  ToolsUpgradeParams,
} from '../types/protocol.ts';
import { STRATA_EVENTS, StrataRpcError } from '../types/rpc.ts';
import { MOCK_TOOL_BUNDLES, MOCK_TOOL_CATALOG } from './mockCatalogData.ts';
import type { MockRuntimeContext } from './mockHandlers.ts';
import { requireMockContainer } from './mockHandlers.ts';
import { launchMockTask, type MockTaskRecord } from './mockTaskHandlers.ts';

const FORBIDDEN_PACKAGES = new Set([
  'openssh-server',
  'openssh-sftp-server-privsep',
  'sshd',
  'sudo',
  'policykit-1',
]);
const PROTECTED_PACKAGES = new Set([
  'dropbear',
  'dropbear-bin',
  'openssh-sftp-server',
  'ca-certificates',
  'libc6',
  'bash',
  'host-exec',
  'strata-debug',
]);

function redactSecret(raw: string): string {
  if (!raw) return '';
  return raw.length <= 8 ? '****' : `${raw.slice(0, 4)}...${raw.slice(-4)}`;
}

export function handleMockToolsRpc(
  ctx: MockRuntimeContext,
  tasks: Map<string, MockTaskRecord>,
  method: string,
  rawParams: unknown,
): unknown {
  const p = (rawParams ?? {}) as Record<string, unknown>;
  switch (method) {
    case 'tools.status': {
      const rec = requireMockContainer(ctx.dataset, p.container as string | undefined);
      rec.tools.status.installedToolCount = rec.tools.items.length;
      rec.tools.status.outdatedToolCount = rec.tools.items.filter((i) => i.outdated).length;
      return rec.tools.status;
    }
    case 'tools.list': {
      const params = p as unknown as ToolsListParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const items = params.group
        ? rec.tools.items.filter((i) => i.group === params.group)
        : rec.tools.items;
      return {
        container: rec.snapshot.id,
        miseAvailable: rec.tools.status.mise.installed,
        uvAvailable: rec.tools.status.uv.installed,
        items,
      };
    }
    case 'tools.catalog': {
      const params = p as unknown as ToolsCatalogParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const q = params.query?.trim().toLowerCase() ?? '';
      const entries = MOCK_TOOL_CATALOG.filter(
        (e) =>
          (!params.group || e.group === params.group) &&
          (!q ||
            e.id.includes(q) ||
            e.name.toLowerCase().includes(q) ||
            e.summary.toLowerCase().includes(q)),
      );
      return { container: rec.snapshot.id, entries, bundles: MOCK_TOOL_BUNDLES };
    }
    case 'tools.configGet':
      return requireMockContainer(ctx.dataset, p.container as string | undefined).tools.config;
    case 'tools.configSet': {
      const params = p as unknown as ToolsConfigSetParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const cfg = rec.tools.config;
      if (params.mirrorPreset) {
        cfg.mirrorPreset = params.mirrorPreset;
        rec.tools.status.mirrorPreset = params.mirrorPreset;
      }
      for (const k of [
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
        'miseConfigToml',
      ] as const) {
        if (params[k] !== undefined) cfg[k] = params[k];
      }
      if (params.jobs !== undefined) cfg.jobs = params.jobs;
      if (params.githubToken !== undefined) {
        const tok = params.githubToken.trim();
        cfg.hasGithubToken = Boolean(tok);
        cfg.githubTokenMasked = tok ? redactSecret(tok) : undefined;
        rec.tools.status.hasGithubToken = cfg.hasGithubToken;
      }
      ctx.emit(STRATA_EVENTS.TOOLS_STATE_CHANGED, {
        container: rec.snapshot.id,
        reason: 'config_updated',
      });
      return cfg;
    }
    case 'tools.bootstrap': {
      const params = p as unknown as ToolsBootstrapParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const profile = params.profile ?? 'dev';
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        ['strata-tools', 'bootstrap', profile],
        [
          '[bootstrap] initializing 4-path standard layout',
          '[bootstrap] mise + uv + layout.json v1 verified',
        ],
        'bootstrap_completed',
        () => {
          rec.tools.status.layoutCompliant = true;
          rec.tools.status.layoutSchemaVersion = 1;
        },
      );
    }
    case 'tools.install': {
      const params = p as unknown as ToolsInstallParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      if (!params.packages.length) throw new StrataRpcError(-32602, '安装包列表不能为空');
      for (const spec of params.packages) {
        const base = spec.split('@')[0]?.split('=')[0]?.trim().toLowerCase() ?? '';
        if (FORBIDDEN_PACKAGES.has(base)) {
          throw new StrataRpcError(-32602, `ColorOS Watchdog 严禁安装 ${spec}`);
        }
      }
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        [params.manager, 'install', ...params.packages],
        [
          `[tools.install] manager=${params.manager} packages=${params.packages.join(', ')}`,
          '[tools.install] shims regenerated',
        ],
        'install_completed',
        () => {
          const targets: Array<[ToolManagerKind, string]> =
            params.manager === 'bundle'
              ? (MOCK_TOOL_BUNDLES.find((b) => b.id === params.packages[0])?.tools ?? []).map(
                  (t) => {
                    const [m, s] = t.split(':');
                    return [(m as ToolManagerKind) || 'mise', s ?? t];
                  },
                )
              : params.packages.map((s) => [params.manager, s]);
          for (const [mgr, spec] of targets) {
            const [name, ver] = spec.split('@');
            if (!name || rec.tools.items.some((i) => i.name === name && i.manager === mgr))
              continue;
            const cat = MOCK_TOOL_CATALOG.find((c) => c.id === name);
            const group: ToolGroupKind = cat?.group ?? (mgr === 'uv' ? 'python-uv' : 'cli-dev');
            rec.tools.items.push({
              id: name,
              name,
              manager: mgr,
              group,
              version: ver || cat?.defaultVersion || '1.0.0',
              outdated: false,
              active: true,
              installed: true,
              broken: false,
              installPath:
                mgr === 'mise' ? `/opt/strata/mise/installs/${name}` : `/usr/bin/${name}`,
            } as InstalledToolItem);
          }
        },
      );
    }
    case 'tools.uninstall': {
      const params = p as unknown as ToolsUninstallParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      for (const pkg of params.packages) {
        if (PROTECTED_PACKAGES.has(pkg.toLowerCase())) {
          throw new StrataRpcError(-32602, `受保护核心包禁止卸载: ${pkg}`);
        }
      }
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        [params.manager, 'uninstall', ...params.packages],
        [`[tools.uninstall] removing ${params.packages.join(', ')}`, '[tools.uninstall] done'],
        'uninstall_completed',
        () => {
          const drop = new Set(params.packages);
          rec.tools.items = rec.tools.items.filter(
            (i) => !(i.manager === params.manager && drop.has(i.name)),
          );
        },
      );
    }
    case 'tools.upgrade': {
      const params = p as unknown as ToolsUpgradeParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const label = params.packages.length ? params.packages.join(', ') : 'all outdated tools';
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        ['tools', 'upgrade', label],
        [`[tools.upgrade] updating ${label}...`, '[tools.upgrade] shims updated'],
        'upgrade_completed',
        () => {
          for (const item of rec.tools.items) {
            if (item.outdated && (!params.packages.length || params.packages.includes(item.name))) {
              item.version = item.latestVersion ?? item.version;
              item.outdated = false;
              item.latestVersion = undefined;
            }
          }
        },
      );
    }
    case 'tools.doctor':
      return requireMockContainer(ctx.dataset, p.container as string | undefined).tools.doctor;
    case 'tools.cacheClean': {
      const params = p as unknown as ToolsCacheCleanParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const beforeBytes = rec.tools.status.cacheBytes;
      const afterBytes = Math.round(beforeBytes * 0.15);
      rec.tools.status.cacheBytes = afterBytes;
      return {
        container: rec.snapshot.id,
        beforeBytes,
        afterBytes,
        freedBytes: beforeBytes - afterBytes,
      };
    }
    default:
      return undefined;
  }
}
