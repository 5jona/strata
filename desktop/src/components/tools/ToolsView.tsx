import { EmptyState, SegmentedControl } from '@gojona/ui';
import { Box } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type { ContainerStatusSnapshot, ToolGroupKind } from '../../types/protocol.ts';
import { TaskStreamDrawer } from './TaskStreamDrawer.tsx';
import { ToolsCatalogTab } from './ToolsCatalogTab.tsx';
import { ToolsConfigTab } from './ToolsConfigTab.tsx';
import { ToolsDoctorTab } from './ToolsDoctorTab.tsx';
import { ToolsInstalledTab } from './ToolsInstalledTab.tsx';
import { ToolsOverviewBar } from './ToolsOverviewBar.tsx';
import { ToolVersionPickerModal, type VersionPickerTarget } from './ToolVersionPickerModal.tsx';
import { useToolsData } from './useToolsData.ts';

type ToolsSubTab = 'installed' | 'catalog' | 'config' | 'doctor';

export interface ToolsViewProps {
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  selectedContainerId?: string;
  onSelectContainer?: (id: string) => void;
  onRefresh?: () => Promise<void> | void;
  onOpenTerminal?: (containerId: string) => void;
  runAction: <T>(
    label: string,
    action: () => Promise<T>,
    successMessage?: string,
  ) => Promise<T | null>;
}

/** Container toolchain workbench with auto-discovery, multi-version switching, and shell presets. */
export function ToolsView({
  transport,
  containers,
  selectedContainerId,
  onSelectContainer,
  onRefresh,
  onOpenTerminal,
  runAction,
}: ToolsViewProps) {
  const [internalContainer, setInternalContainer] = useState(containers[0]?.id ?? 'main');
  const selectedContainer = selectedContainerId ?? internalContainer;
  const setSelectedContainer = (id: string) => {
    setInternalContainer(id);
    onSelectContainer?.(id);
  };

  const [subTab, setSubTab] = useState<ToolsSubTab>('installed');
  const [groupFilter, setGroupFilter] = useState<'all' | ToolGroupKind>('all');
  const [checkOutdated, setCheckOutdated] = useState(false);
  const [catalogQuery, setCatalogQuery] = useState('');
  const [configPathOverride, setConfigPathOverride] = useState<string | undefined>(undefined);
  const [pickerTarget, setPickerTarget] = useState<VersionPickerTarget | null>(null);

  useEffect(() => {
    if (
      !selectedContainerId &&
      containers.length > 0 &&
      !containers.some((c) => c.id === selectedContainer)
    ) {
      setInternalContainer(containers[0].id);
    }
  }, [containers, selectedContainer, selectedContainerId]);

  const activeContainerRunning =
    containers.find((c) => c.id === selectedContainer)?.phase === 'running';

  const data = useToolsData(
    transport,
    selectedContainer,
    containers.length,
    activeContainerRunning,
    checkOutdated,
    catalogQuery,
    runAction,
    onRefresh,
    configPathOverride,
  );

  const stableConfigPaths = useMemo(
    () => data.config?.configPaths ?? data.status?.miseConfigPaths ?? [],
    [data.config?.configPaths, data.status?.miseConfigPaths],
  );

  if (containers.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <EmptyState
          icon={<Box className="size-10" />}
          title="暂无可用容器"
          description="请先前往「安装与导入」创建或导入至少一个 Linux 容器，随后即可管理其工具链与运行环境。"
        />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-3 overflow-hidden p-5">
      <ToolsOverviewBar
        containers={containers}
        selectedContainer={selectedContainer}
        activeContainerRunning={activeContainerRunning}
        status={data.status}
        busy={data.busy}
        disabled={!transport}
        onSelectContainer={(id) => {
          setConfigPathOverride(undefined);
          setSelectedContainer(id);
        }}
        onStartContainer={data.startContainer}
        onBootstrap={data.bootstrapTools}
        onCleanCache={data.cleanCache}
        onRefresh={() => void data.loadOverview()}
      />

      <div className="shrink-0">
        <SegmentedControl
          value={subTab}
          onValueChange={setSubTab}
          options={[
            {
              value: 'installed',
              label: `已安装与版本管理 (${data.listResult?.items.length ?? 0})`,
            },
            { value: 'catalog', label: '安装工具与终端预设' },
            { value: 'config', label: '源、代理与配置' },
            { value: 'doctor', label: '环境体检' },
          ]}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto pr-1 strata-scroll">
        {subTab === 'installed' && (
          <ToolsInstalledTab
            listResult={data.listResult}
            groupFilter={groupFilter}
            checkOutdated={checkOutdated}
            containerRunning={activeContainerRunning}
            busy={data.busy}
            onGroupFilterChange={setGroupFilter}
            onCheckOutdatedChange={setCheckOutdated}
            onUpgradeAll={data.upgradeAllTools}
            onUpgradeTool={data.upgradeTool}
            onUninstallVersion={data.uninstallToolVersion}
            onUseVersion={data.switchToolVersion}
            onOpenVersionPicker={(mode, manager, name, ver) =>
              setPickerTarget({ mode, manager, name, ...(ver ? { defaultVersion: ver } : {}) })
            }
          />
        )}
        {subTab === 'catalog' && (
          <ToolsCatalogTab
            containerId={selectedContainer}
            catalog={data.catalog}
            containerRunning={activeContainerRunning}
            busy={data.busy}
            onSearchCatalog={setCatalogQuery}
            onInstall={data.installTools}
            onOpenVersionPicker={(manager, name, defaultVersion) =>
              setPickerTarget({ mode: 'install', manager, name, defaultVersion })
            }
            {...(data.status?.systemPkgManager
              ? { systemPkgManager: data.status.systemPkgManager }
              : {})}
            {...(onOpenTerminal ? { onOpenTerminal } : {})}
          />
        )}
        {subTab === 'config' && (
          <ToolsConfigTab
            config={data.config}
            containerRunning={activeContainerRunning}
            busy={data.busy}
            onSelectConfigPath={setConfigPathOverride}
            onSave={data.saveConfig}
          />
        )}
        {subTab === 'doctor' && (
          <ToolsDoctorTab
            doctor={data.doctor}
            busy={data.busy}
            onRunDoctor={() => void data.loadOverview()}
          />
        )}
      </div>

      <TaskStreamDrawer
        tasks={data.tasks}
        activeTaskId={data.activeTaskId}
        expanded={data.drawerOpen}
        onSelectTask={data.setActiveTaskId}
        onToggleExpand={() => data.setDrawerOpen((v) => !v)}
        onCancel={data.cancelTask}
        onClearFinished={() => data.setTasks((prev) => prev.filter((t) => t.running))}
      />

      <ToolVersionPickerModal
        target={pickerTarget}
        container={selectedContainer}
        transport={transport}
        configPaths={stableConfigPaths}
        busy={data.busy}
        onClose={() => setPickerTarget(null)}
        onInstallVersion={(manager, spec, pin) => data.installTools(manager, [spec], pin)}
        onUseVersion={data.switchToolVersion}
      />
    </div>
  );
}
