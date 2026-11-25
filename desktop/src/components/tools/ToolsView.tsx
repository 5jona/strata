import { SegmentedControl, useToast } from '@gojona/ui';
import { useCallback, useEffect, useState } from 'react';
import { formatBytes } from '../../lib/format.ts';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type {
  ContainerStatusSnapshot,
  InstalledToolItem,
  ToolGroupKind,
  ToolManagerKind,
  ToolsBootstrapProfile,
  ToolsCatalogResult,
  ToolsConfigResult,
  ToolsConfigSetParams,
  ToolsDoctorResult,
  ToolsListResult,
  ToolsStatusResult,
} from '../../types/protocol.ts';
import { STRATA_EVENTS, STRATA_METHODS } from '../../types/rpc.ts';
import { type ActiveToolTaskState, TaskStreamDrawer } from './TaskStreamDrawer.tsx';
import { ToolsCatalogTab } from './ToolsCatalogTab.tsx';
import { ToolsConfigTab } from './ToolsConfigTab.tsx';
import { ToolsDoctorTab } from './ToolsDoctorTab.tsx';
import { ToolsInstalledTab } from './ToolsInstalledTab.tsx';
import { ToolsOverviewBar } from './ToolsOverviewBar.tsx';

type ToolsSubTab = 'installed' | 'catalog' | 'config' | 'doctor';

export interface ToolsViewProps {
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Container toolchain workbench managing `mise`, `uv`, `apt`, mirrors, diagnostics, and live tasks.
 */
export function ToolsView({ transport, containers, runAction }: ToolsViewProps) {
  const { pushToast } = useToast();
  const [selectedContainer, setSelectedContainer] = useState(containers[0]?.id ?? 'main');
  const [subTab, setSubTab] = useState<ToolsSubTab>('installed');
  const [profile, setProfile] = useState<ToolsBootstrapProfile>('dev');
  const [groupFilter, setGroupFilter] = useState<'all' | ToolGroupKind>('all');
  const [checkOutdated, setCheckOutdated] = useState(false);
  const [busy, setBusy] = useState(false);

  const [status, setStatus] = useState<ToolsStatusResult | null>(null);
  const [listResult, setListResult] = useState<ToolsListResult | null>(null);
  const [catalog, setCatalog] = useState<ToolsCatalogResult | null>(null);
  const [config, setConfig] = useState<ToolsConfigResult | null>(null);
  const [doctor, setDoctor] = useState<ToolsDoctorResult | null>(null);
  const [activeTask, setActiveTask] = useState<ActiveToolTaskState | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(true);

  useEffect(() => {
    if (containers.length > 0 && !containers.some((c) => c.id === selectedContainer)) {
      setSelectedContainer(containers[0].id);
    }
  }, [containers, selectedContainer]);

  const loadOverview = useCallback(async () => {
    if (!transport || !selectedContainer) return;
    setBusy(true);
    try {
      const [st, cat] = await Promise.all([
        transport.invoke(STRATA_METHODS.TOOLS_STATUS, { container: selectedContainer }),
        transport.invoke(STRATA_METHODS.TOOLS_CATALOG, { container: selectedContainer }),
      ]);
      setStatus(st);
      setCatalog(cat);
      const [ls, cfg, doc] = await Promise.all([
        transport
          .invoke(STRATA_METHODS.TOOLS_LIST, { container: selectedContainer, checkOutdated })
          .catch(() => null),
        transport
          .invoke(STRATA_METHODS.TOOLS_CONFIG_GET, { container: selectedContainer })
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
  }, [checkOutdated, selectedContainer, transport]);

  useEffect(() => {
    void loadOverview();
  }, [loadOverview]);

  useEffect(() => {
    if (!transport) return;
    return transport.subscribe((env) => {
      if (env.event === STRATA_EVENTS.TASK_OUTPUT) {
        setActiveTask((prev) =>
          prev && prev.taskId === env.payload.taskId
            ? { ...prev, entries: [...prev.entries, env.payload] }
            : prev,
        );
      } else if (env.event === STRATA_EVENTS.TASK_EXIT) {
        setActiveTask((prev) =>
          prev && prev.taskId === env.payload.taskId
            ? { ...prev, running: false, exit: env.payload }
            : prev,
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
    fn: () => Promise<{ taskId: string; container: string }>,
  ) => {
    const res = await runAction(label, fn);
    if (!res) return;
    setActiveTask({
      taskId: res.taskId,
      container: res.container,
      label,
      running: true,
      entries: [],
    });
    setDrawerOpen(true);
  };

  const handleInstall = (manager: ToolManagerKind, packages: string[], pin: boolean) => {
    if (!transport) return;
    void startTask(`安装 ${packages.join(', ')}`, () =>
      transport.invoke(STRATA_METHODS.TOOLS_INSTALL, {
        container: selectedContainer,
        manager,
        packages,
        pin,
      }),
    );
  };

  const handleUpgrade = (packages: string[], manager?: ToolManagerKind) => {
    if (!transport) return;
    void startTask(packages.length ? `升级 ${packages.join(', ')}` : '升级全部过期工具', () =>
      transport.invoke(STRATA_METHODS.TOOLS_UPGRADE, {
        container: selectedContainer,
        ...(manager ? { manager } : {}),
        packages,
        bump: false,
      }),
    );
  };

  const handleUninstall = (item: InstalledToolItem) => {
    if (!transport) return;
    void startTask(`卸载 ${item.name}`, () =>
      transport.invoke(STRATA_METHODS.TOOLS_UNINSTALL, {
        container: selectedContainer,
        manager: item.manager,
        packages: [item.name],
      }),
    );
  };

  const handleSaveConfig = async (params: ToolsConfigSetParams) => {
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

  const handleCleanCache = () => {
    if (!transport) return;
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
  };

  return (
    <div className="flex h-full flex-col gap-3 overflow-hidden p-5">
      <ToolsOverviewBar
        containers={containers}
        selectedContainer={selectedContainer}
        profile={profile}
        status={status}
        busy={busy}
        disabled={!transport}
        onSelectContainer={setSelectedContainer}
        onSelectProfile={setProfile}
        onBootstrap={() =>
          transport &&
          void startTask(`规范化初始化 (${profile})`, () =>
            transport.invoke(STRATA_METHODS.TOOLS_BOOTSTRAP, {
              container: selectedContainer,
              profile,
            }),
          )
        }
        onCleanCache={handleCleanCache}
        onRefresh={() => void loadOverview()}
      />

      <div className="shrink-0">
        <SegmentedControl
          value={subTab}
          onValueChange={setSubTab}
          options={[
            { value: 'installed', label: `已安装工具 (${listResult?.items.length ?? 0})` },
            { value: 'catalog', label: '工具目录与安装' },
            { value: 'config', label: '镜像源与配置' },
            { value: 'doctor', label: '环境体检 (Doctor)' },
          ]}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto pr-1 strata-scroll">
        {subTab === 'installed' && (
          <ToolsInstalledTab
            listResult={listResult}
            groupFilter={groupFilter}
            checkOutdated={checkOutdated}
            busy={busy}
            onGroupFilterChange={setGroupFilter}
            onCheckOutdatedChange={setCheckOutdated}
            onUpgradeAll={() => handleUpgrade([])}
            onUpgradeTool={(item) => handleUpgrade([item.name], item.manager)}
            onUninstallTool={handleUninstall}
          />
        )}
        {subTab === 'catalog' && (
          <ToolsCatalogTab catalog={catalog} busy={busy} onInstall={handleInstall} />
        )}
        {subTab === 'config' && (
          <ToolsConfigTab config={config} busy={busy} onSave={handleSaveConfig} />
        )}
        {subTab === 'doctor' && (
          <ToolsDoctorTab doctor={doctor} busy={busy} onRunDoctor={() => void loadOverview()} />
        )}
      </div>

      <TaskStreamDrawer
        task={activeTask}
        expanded={drawerOpen}
        onToggleExpand={() => setDrawerOpen((v) => !v)}
        onCancel={(taskId) =>
          transport &&
          void runAction('取消任务', () => transport.invoke(STRATA_METHODS.TASK_CANCEL, { taskId }))
        }
        onClear={() => setActiveTask(null)}
      />
    </div>
  );
}
