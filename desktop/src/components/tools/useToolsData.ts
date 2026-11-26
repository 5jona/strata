import { useToast } from '@gojona/ui';
import { useCallback, useEffect, useState } from 'react';
import { formatBytes } from '../../lib/format.ts';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type {
  InstalledToolItem,
  ToolManagerKind,
  ToolMirrorPreset,
  ToolsBootstrapProfile,
  ToolsCatalogResult,
  ToolsConfigResult,
  ToolsConfigSetParams,
  ToolsDoctorResult,
  ToolsListResult,
  ToolsStatusResult,
  ToolUseScope,
} from '../../types/protocol.ts';
import { STRATA_EVENTS, STRATA_METHODS } from '../../types/rpc.ts';
import type { ActiveToolTaskState } from './TaskStreamDrawer.tsx';

export type ToolsRunAction = <T>(
  label: string,
  action: () => Promise<T>,
  successMessage?: string,
) => Promise<T | null>;

/** Internal data, action dispatch, and task-stream hook for `ToolsView`. */
export function useToolsData(
  transport: StrataTransport | null,
  selectedContainer: string,
  containerCount: number,
  activeContainerRunning: boolean,
  checkOutdated: boolean,
  catalogQuery: string,
  runAction: ToolsRunAction,
  onRefresh?: () => Promise<void> | void,
  configPathOverride?: string,
) {
  const { pushToast } = useToast();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<ToolsStatusResult | null>(null);
  const [listResult, setListResult] = useState<ToolsListResult | null>(null);
  const [catalog, setCatalog] = useState<ToolsCatalogResult | null>(null);
  const [config, setConfig] = useState<ToolsConfigResult | null>(null);
  const [doctor, setDoctor] = useState<ToolsDoctorResult | null>(null);
  const [tasks, setTasks] = useState<ActiveToolTaskState[]>([]);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(true);

  const loadCatalog = useCallback(async () => {
    if (!transport || !selectedContainer || containerCount === 0) return;
    try {
      const cat = await transport.invoke(STRATA_METHODS.TOOLS_CATALOG, {
        container: selectedContainer,
        ...(catalogQuery.trim() ? { query: catalogQuery.trim() } : {}),
      });
      setCatalog(cat);
    } catch {
      // Keep previous catalog snapshot on transient query error
    }
  }, [catalogQuery, containerCount, selectedContainer, transport]);

  const loadOverview = useCallback(async () => {
    if (!transport || !selectedContainer || containerCount === 0) return;
    setBusy(true);
    try {
      const st = await transport.invoke(STRATA_METHODS.TOOLS_STATUS, {
        container: selectedContainer,
      });
      setStatus(st);
      if (!activeContainerRunning) {
        setListResult(null);
        setConfig(null);
        setDoctor(null);
        return;
      }
      const [ls, cfg, doc] = await Promise.all([
        transport
          .invoke(STRATA_METHODS.TOOLS_LIST, { container: selectedContainer, checkOutdated })
          .catch(() => null),
        transport
          .invoke(STRATA_METHODS.TOOLS_CONFIG_GET, {
            container: selectedContainer,
            ...(configPathOverride ? { configPath: configPathOverride } : {}),
          })
          .catch(() => null),
        transport
          .invoke(STRATA_METHODS.TOOLS_DOCTOR, { container: selectedContainer })
          .catch(() => null),
      ]);
      if (ls) setListResult(ls);
      if (cfg) setConfig(cfg);
      if (doc) setDoctor(doc);
    } finally {
      setBusy(false);
    }
  }, [
    activeContainerRunning,
    checkOutdated,
    configPathOverride,
    containerCount,
    selectedContainer,
    transport,
  ]);

  useEffect(() => {
    void loadOverview();
  }, [loadOverview]);

  useEffect(() => {
    void loadCatalog();
  }, [loadCatalog]);

  useEffect(() => {
    if (!transport) return;
    return transport.subscribe((env) => {
      if (env.event === STRATA_EVENTS.TASK_OUTPUT) {
        setTasks((prev) =>
          prev.map((t) =>
            t.taskId === env.payload.taskId ? { ...t, entries: [...t.entries, env.payload] } : t,
          ),
        );
      } else if (env.event === STRATA_EVENTS.TASK_EXIT) {
        setTasks((prev) =>
          prev.map((t) =>
            t.taskId === env.payload.taskId ? { ...t, running: false, exit: env.payload } : t,
          ),
        );
      } else if (
        env.event === STRATA_EVENTS.TOOLS_STATE_CHANGED &&
        env.payload.container === selectedContainer
      ) {
        void loadOverview();
      }
    });
  }, [loadOverview, selectedContainer, transport]);

  const startTask = async (
    label: string,
    fn: (t: StrataTransport) => Promise<{ taskId: string; container: string }>,
  ) => {
    if (!transport) return;
    const res = await runAction(label, () => fn(transport));
    if (!res) return;
    const nextTask: ActiveToolTaskState = {
      taskId: res.taskId,
      container: res.container,
      label,
      running: true,
      entries: [],
    };
    setTasks((prev) => [nextTask, ...prev].slice(0, 8));
    setActiveTaskId(res.taskId);
    setDrawerOpen(true);
  };

  const installTools = (manager: ToolManagerKind, packages: string[], pin: boolean) =>
    void startTask(`安装 ${packages.join(', ')}`, (t) =>
      t.invoke(STRATA_METHODS.TOOLS_INSTALL, {
        container: selectedContainer,
        manager,
        packages,
        pin,
      }),
    );

  const switchToolVersion = (
    manager: ToolManagerKind,
    name: string,
    version: string,
    scope: ToolUseScope,
    configPath?: string,
  ) =>
    void startTask(`切换 ${name}@${version}`, (t) =>
      t.invoke(STRATA_METHODS.TOOLS_USE, {
        container: selectedContainer,
        manager,
        name,
        version,
        scope,
        ...(configPath ? { configPath } : {}),
      }),
    );

  const uninstallToolVersion = (item: InstalledToolItem) => {
    const pkgSpec =
      item.manager === 'mise' || item.backend === 'uv-python'
        ? `${item.name}@${item.version}`
        : item.name;
    void startTask(`卸载 ${pkgSpec}`, (t) =>
      t.invoke(STRATA_METHODS.TOOLS_UNINSTALL, {
        container: selectedContainer,
        manager: item.manager,
        packages: [pkgSpec],
      }),
    );
  };

  const upgradeAllTools = () =>
    void startTask('升级全部过期工具', (t) =>
      t.invoke(STRATA_METHODS.TOOLS_UPGRADE, {
        container: selectedContainer,
        packages: [],
        bump: false,
      }),
    );

  const upgradeTool = (item: InstalledToolItem) =>
    void startTask(`升级 ${item.name}`, (t) =>
      t.invoke(STRATA_METHODS.TOOLS_UPGRADE, {
        container: selectedContainer,
        manager: item.manager,
        packages: [item.name],
        bump: false,
      }),
    );

  const startContainer = () =>
    transport &&
    void runAction(`启动容器 ${selectedContainer}`, async () => {
      await transport.invoke(STRATA_METHODS.CONTAINER_START, { container: selectedContainer });
      pushToast(`容器 ${selectedContainer} 已启动`, 'success');
      await onRefresh?.();
    });

  const bootstrapTools = (profile: ToolsBootstrapProfile, mirrorPreset: ToolMirrorPreset) =>
    void startTask(`初始化工具链 (${profile})`, (t) =>
      t.invoke(STRATA_METHODS.TOOLS_BOOTSTRAP, {
        container: selectedContainer,
        profile,
        mirrorPreset,
      }),
    );

  const cleanCache = () =>
    transport &&
    void runAction('清理工具缓存', async () => {
      const res = await transport.invoke(STRATA_METHODS.TOOLS_CACHE_CLEAN, {
        container: selectedContainer,
        cleanMise: true,
        cleanUv: true,
        cleanApt: true,
        pruneUnused: false,
      });
      pushToast(`已释放缓存 ${formatBytes(res.freedBytes)}`, 'success');
      await loadOverview();
    });

  const saveConfig = async (params: ToolsConfigSetParams) => {
    if (!transport) return;
    setBusy(true);
    const updated = await runAction('保存镜像与工具配置', () =>
      transport.invoke(STRATA_METHODS.TOOLS_CONFIG_SET, params),
    );
    if (updated) {
      setConfig(updated);
      pushToast('工具环境配置已更新', 'success');
    }
    setBusy(false);
  };

  const cancelTask = (taskId: string) =>
    transport &&
    void runAction('取消任务', () => transport.invoke(STRATA_METHODS.TASK_CANCEL, { taskId }));

  return {
    busy,
    status,
    listResult,
    catalog,
    config,
    doctor,
    tasks,
    setTasks,
    activeTaskId,
    setActiveTaskId,
    drawerOpen,
    setDrawerOpen,
    loadOverview,
    installTools,
    switchToolVersion,
    uninstallToolVersion,
    upgradeAllTools,
    upgradeTool,
    startContainer,
    bootstrapTools,
    cleanCache,
    saveConfig,
    cancelTask,
  };
}
