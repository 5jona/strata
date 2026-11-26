import type {
  ToolsBootstrapParams,
  ToolsCacheCleanParams,
  ToolsCatalogParams,
  ToolsConfigGetParams,
  ToolsConfigSetParams,
  ToolsInstallParams,
  ToolsListParams,
  ToolsUninstallParams,
  ToolsUpgradeParams,
  ToolsUseParams,
  ToolsVersionsParams,
} from '../types/protocol.ts';
import { STRATA_EVENTS, StrataRpcError } from '../types/rpc.ts';
import {
  MOCK_MISE_REGISTRY_EXTRAS,
  MOCK_TOOL_BUNDLES,
  MOCK_TOOL_CATALOG,
} from './mockCatalogData.ts';
import { type MockRuntimeContext, requireMockContainer } from './mockHandlers.ts';
import {
  applyMockToolInstall,
  applyMockToolUse,
  launchMockTask,
  type MockTaskRecord,
  resolveMockVersions,
  splitMockToolSpec,
} from './mockTaskHandlers.ts';
import type { MockToolsState } from './mockToolsInitialState.ts';
import { applyMockMirrorPreset, applyMockToolsConfigSet } from './mockToolsInitialState.ts';

const FORBIDDEN_PACKAGES = new Set([
  'openssh',
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
      const pool = q ? [...MOCK_TOOL_CATALOG, ...MOCK_MISE_REGISTRY_EXTRAS] : MOCK_TOOL_CATALOG;
      const entries = pool.filter(
        (e) =>
          (!params.group || e.group === params.group) &&
          (!q ||
            e.id.toLowerCase().includes(q) ||
            e.name.toLowerCase().includes(q) ||
            e.summary.toLowerCase().includes(q) ||
            e.tags.some((t) => t.toLowerCase().includes(q))),
      );
      return {
        container: rec.snapshot.id,
        registryAvailable: rec.tools.status.mise.installed,
        entries,
        bundles: MOCK_TOOL_BUNDLES,
      };
    }
    case 'tools.configGet': {
      const params = p as unknown as ToolsConfigGetParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const activePath =
        params.configPath || rec.tools.config.configPath || rec.tools.config.configPaths[0];
      if (activePath) {
        rec.tools.config.configPath = activePath;
        rec.tools.config.miseConfigToml =
          rec.tools.configFiles[activePath] ?? rec.tools.config.miseConfigToml;
      }
      return rec.tools.config;
    }
    case 'tools.configSet': {
      const rec = requireMockContainer(
        ctx.dataset,
        (p as unknown as ToolsConfigSetParams).container,
      );
      const cfg = applyMockToolsConfigSet(
        rec.tools as MockToolsState,
        p as unknown as ToolsConfigSetParams,
      );
      ctx.emit(STRATA_EVENTS.TOOLS_STATE_CHANGED, {
        container: rec.snapshot.id,
        reason: 'config_updated',
      });
      return cfg;
    }
    case 'tools.versions':
      return resolveMockVersions(
        requireMockContainer(ctx.dataset, (p as unknown as ToolsVersionsParams).container),
        p as unknown as ToolsVersionsParams,
      );
    case 'tools.use': {
      const params = p as unknown as ToolsUseParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const targetCfg =
        params.configPath ||
        (params.scope === 'workspace'
          ? '/workspace/mise.toml'
          : (rec.tools.config.configPath ?? '/root/.config/mise/config.toml'));
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        [params.manager, 'use', `${params.name}@${params.version}`],
        [
          `[tools.use] ${params.name}@${params.version} -> ${targetCfg}`,
          '[tools.use] shims regenerated',
        ],
        'use_completed',
        () => applyMockToolUse(rec, params, targetCfg),
      );
    }
    case 'tools.bootstrap': {
      const params = p as unknown as ToolsBootstrapParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      const profile = params.profile ?? 'dev';
      if (params.mirrorPreset) applyMockMirrorPreset(rec.tools.config, params.mirrorPreset);
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        ['tools', 'bootstrap', profile],
        [`[bootstrap] profile=${profile}`, '[bootstrap] mise & uv ready'],
        'bootstrap_completed',
        () => {
          rec.tools.status.mise.installed = true;
          rec.tools.status.uv.installed = true;
          rec.tools.status.layoutCompliant = true;
        },
      );
    }
    case 'tools.install': {
      const params = p as unknown as ToolsInstallParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      if (!params.packages.length) throw new StrataRpcError(-32602, '安装列表不能为空');
      for (const spec of params.packages) {
        const [base] = splitMockToolSpec(spec.split('=')[0]?.trim().toLowerCase() ?? '');
        if (FORBIDDEN_PACKAGES.has(base))
          throw new StrataRpcError(-32602, `安全策略禁止安装 ${spec}`);
      }
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        [params.manager, 'install', ...params.packages],
        [
          `[tools.install] manager=${params.manager} packages=${params.packages.join(', ')}`,
          '[tools.install] done',
        ],
        'install_completed',
        () => applyMockToolInstall(rec, params),
      );
    }
    case 'tools.uninstall': {
      const params = p as unknown as ToolsUninstallParams;
      const rec = requireMockContainer(ctx.dataset, params.container);
      for (const pkg of params.packages) {
        const [base] = splitMockToolSpec(pkg.toLowerCase());
        if (PROTECTED_PACKAGES.has(base))
          throw new StrataRpcError(-32602, `受保护核心包禁止卸载: ${pkg}`);
      }
      return launchMockTask(
        ctx,
        tasks,
        rec.snapshot.id,
        [params.manager, 'uninstall', ...params.packages],
        [`[tools.uninstall] removing ${params.packages.join(', ')}`, '[tools.uninstall] done'],
        'uninstall_completed',
        () => {
          for (const spec of params.packages) {
            const [name, ver] = splitMockToolSpec(spec);
            rec.tools.items = rec.tools.items.filter(
              (i) =>
                !(i.manager === params.manager && i.name === name && (!ver || i.version === ver)),
            );
          }
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
