import type {
  InstalledToolItem,
  TaskOutputPayload,
  TaskRunResult,
  TaskSummary,
  ToolGroupKind,
  ToolManagerKind,
  ToolsInstallParams,
  ToolsUseParams,
  ToolsVersionsParams,
  ToolVersionOption,
} from '../types/protocol.ts';
import { STRATA_EVENTS, StrataRpcError } from '../types/rpc.ts';
import {
  MOCK_MISE_REGISTRY_EXTRAS,
  MOCK_TOOL_BUNDLES,
  MOCK_TOOL_CATALOG,
  MOCK_TOOL_VERSIONS_MAP,
} from './mockCatalogData.ts';
import type { MockContainerRecord } from './mockData.ts';
import type { MockRuntimeContext } from './mockHandlers.ts';

export interface MockTaskRecord {
  summary: TaskSummary;
  entries: TaskOutputPayload[];
  cancelled: boolean;
}

export function splitMockToolSpec(spec: string): [string, string | undefined] {
  const idx = spec.lastIndexOf('@');
  if (idx > 0 && !spec.slice(0, idx).endsWith(':')) {
    return [spec.slice(0, idx), spec.slice(idx + 1) || undefined];
  }
  return [spec, undefined];
}

export function launchMockTask(
  ctx: MockRuntimeContext,
  tasks: Map<string, MockTaskRecord>,
  container: string,
  command: string[],
  lines: string[],
  reason: string,
  onComplete: () => void,
): TaskRunResult {
  const taskId = `task-${Date.now().toString(36)}-${Math.floor(Math.random() * 999)}`;
  const record: MockTaskRecord = {
    summary: {
      taskId,
      container,
      pid: 24_100 + tasks.size,
      command,
      running: true,
      startedAtMs: Date.now(),
    },
    entries: [],
    cancelled: false,
  };
  tasks.set(taskId, record);

  lines.forEach((line, idx) => {
    ctx.schedule(140 * (idx + 1), () => {
      if (record.cancelled) return;
      const entry: TaskOutputPayload = {
        taskId,
        container,
        stream: 'stdout',
        chunk: `${line}\n`,
        seq: idx + 1,
      };
      record.entries.push(entry);
      ctx.emit(STRATA_EVENTS.TASK_OUTPUT, entry);
    });
  });

  ctx.schedule(140 * (lines.length + 1), () => {
    if (record.cancelled) return;
    onComplete();
    record.summary.running = false;
    record.summary.exitCode = 0;
    ctx.emit(STRATA_EVENTS.TASK_EXIT, { taskId, container, exitCode: 0, timedOut: false });
    ctx.emit(STRATA_EVENTS.TOOLS_STATE_CHANGED, { container, reason, taskId });
  });

  return { taskId, container, pid: record.summary.pid };
}

export function resolveMockVersions(rec: MockContainerRecord, params: ToolsVersionsParams) {
  const baseList: ToolVersionOption[] = MOCK_TOOL_VERSIONS_MAP[params.name] ?? [
    {
      version: '2.1.0-rc1',
      createdAt: '2026-11-20',
      installed: false,
      active: false,
      prerelease: true,
    },
    {
      version: '2.0.0',
      createdAt: '2026-11-01',
      installed: false,
      active: false,
      prerelease: false,
    },
    {
      version: '1.9.0',
      createdAt: '2026-09-15',
      installed: false,
      active: false,
      prerelease: false,
    },
  ];
  const installedItems = rec.tools.items.filter(
    (i) => i.manager === params.manager && i.name === params.name,
  );
  const merged = baseList
    .filter((v) => params.includePrerelease || !v.prerelease)
    .map((v) => {
      const match = installedItems.find((i) => i.version === v.version);
      return { ...v, installed: Boolean(match), active: Boolean(match?.active) };
    });
  for (const inst of installedItems) {
    if (!merged.some((v) => v.version === inst.version)) {
      merged.unshift({
        version: inst.version,
        installed: true,
        active: inst.active,
        prerelease: false,
      });
    }
  }
  return {
    container: rec.snapshot.id,
    manager: params.manager,
    name: params.name,
    versions: merged,
  };
}

export function applyMockToolUse(
  rec: MockContainerRecord,
  params: ToolsUseParams,
  targetCfg: string,
) {
  let found = false;
  for (const item of rec.tools.items) {
    if (item.manager === params.manager && item.name === params.name) {
      const isTarget = item.version === params.version;
      item.active = isTarget;
      if (isTarget) {
        found = true;
        item.requestedVersion = params.version;
        item.sourcePath = targetCfg;
      }
    }
  }
  if (!found) {
    const cat = MOCK_TOOL_CATALOG.find((c) => c.id === params.name);
    rec.tools.items.push({
      id: `${params.manager}:${params.name}@${params.version}`,
      name: params.name,
      manager: params.manager,
      group: cat?.group ?? 'cli-dev',
      version: params.version,
      requestedVersion: params.version,
      outdated: false,
      sourcePath: targetCfg,
      source: 'user-installed',
      active: true,
      installed: true,
      broken: false,
      installPath: `/root/.local/share/mise/installs/${params.name}/${params.version}`,
    });
  }
}

export function applyMockToolInstall(rec: MockContainerRecord, params: ToolsInstallParams) {
  const targets: Array<[ToolManagerKind, string]> =
    params.manager === 'bundle'
      ? (MOCK_TOOL_BUNDLES.find((b) => b.id === params.packages[0])?.tools ?? []).map((t) => {
          const [m, s] = t.split(':');
          return [(m as ToolManagerKind) || 'mise', s ?? t];
        })
      : params.packages.map((s) => [params.manager, s]);
  for (const [mgr, spec] of targets) {
    const [name, ver] = splitMockToolSpec(spec);
    const resolvedVer = ver && ver !== 'latest' && ver !== 'lts' ? ver : '1.0.0';
    if (
      !name ||
      rec.tools.items.some((i) => i.name === name && i.manager === mgr && i.version === resolvedVer)
    ) {
      continue;
    }
    const cat = [...MOCK_TOOL_CATALOG, ...MOCK_MISE_REGISTRY_EXTRAS].find((c) => c.id === name);
    const group: ToolGroupKind = cat?.group ?? (mgr === 'uv' ? 'python-uv' : 'cli-dev');
    if (params.pin) {
      for (const existing of rec.tools.items) {
        if (existing.manager === mgr && existing.name === name) existing.active = false;
      }
    }
    rec.tools.items.push({
      id: `${mgr}:${name}@${resolvedVer}`,
      name,
      manager: mgr,
      group,
      version: resolvedVer,
      requestedVersion: params.pin ? (ver ?? 'latest') : undefined,
      outdated: false,
      source: mgr === 'apt' ? 'system' : 'user-installed',
      sourcePath: params.pin ? rec.tools.config.configPath : undefined,
      active: params.pin,
      installed: true,
      broken: false,
      installPath:
        mgr === 'mise'
          ? `/root/.local/share/mise/installs/${name}/${resolvedVer}`
          : `/usr/bin/${name}`,
    } as InstalledToolItem);
  }
}

export function handleMockTaskRpc(
  ctx: MockRuntimeContext,
  tasks: Map<string, MockTaskRecord>,
  method: string,
  rawParams: unknown,
): unknown {
  const p = (rawParams ?? {}) as Record<string, unknown>;
  switch (method) {
    case 'task.list':
      return Array.from(tasks.values()).map((t) => t.summary);
    case 'task.logs': {
      const rec = tasks.get(String(p.taskId));
      if (!rec) throw new StrataRpcError(-32025, '任务不存在');
      return { summary: rec.summary, entries: rec.entries };
    }
    case 'task.cancel': {
      const rec = tasks.get(String(p.taskId));
      if (!rec?.summary.running) return { cancelled: false };
      rec.cancelled = true;
      rec.summary.running = false;
      rec.summary.exitCode = 130;
      ctx.emit(STRATA_EVENTS.TASK_EXIT, {
        taskId: rec.summary.taskId,
        container: rec.summary.container,
        exitCode: 130,
        signal: 15,
        timedOut: false,
      });
      return { cancelled: true };
    }
    case 'task.run': {
      const container = String(p.container ?? 'main');
      const command = Array.isArray(p.command) ? p.command.map(String) : ['sh'];
      return launchMockTask(
        ctx,
        tasks,
        container,
        command,
        [`$ ${command.join(' ')}`, 'ok'],
        'task_completed',
        () => {},
      );
    }
    default:
      return undefined;
  }
}
