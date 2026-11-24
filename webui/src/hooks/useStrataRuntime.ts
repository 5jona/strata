import { useSideVeilPlugin } from '@gojona/sideveil-sdk';
import { type LogEntry, useToast } from '@gojona/ui';
import { useCallback, useRef, useState } from 'react';
import type { ServiceControlAction } from '../components/ServicesPanel.tsx';
import {
  type ContainerConfig,
  type ContainerConfigPatchParams,
  type ContainerServiceRuntime,
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  type HudStateResult,
  STRATA_PROTOCOL,
  type TaskSummary,
  type WebStatusResult,
} from '../types.ts';
import {
  type ContainerLifecycleAction,
  type ContainerMaintenanceOp,
  useStrataActions,
} from './useStrataActions.ts';
import { useStrataConfig } from './useStrataConfig.ts';
import { type StrataNotifyFn, useStrataContainers } from './useStrataContainers.ts';
import { useStrataServicesAndWeb } from './useStrataServicesAndWeb.ts';
import { useStrataTasks } from './useStrataTasks.ts';

/** Unified runtime state and command surface consumed by `StrataManagerApp`. */
export interface StrataRuntimeApi {
  containers: ContainerStatusSnapshot[];
  selectedId: string;
  setSelectedId: (id: string) => void;
  activeContainer: ContainerStatusSnapshot | null;
  daemonSummary: DaemonSummarySnapshot | null;
  hudState: HudStateResult;
  busyAction: string | null;
  loading: boolean;
  lastError: string | null;
  tasks: TaskSummary[];
  activeTaskId: string | null;
  taskEntries: LogEntry[];
  dropbearLogs: LogEntry[];
  containerConfig: ContainerConfig | null;
  sshKeys: string[];
  services: ContainerServiceRuntime[];
  serviceLogs: Record<string, string[]>;
  webStatus: WebStatusResult | null;
  pairingToken: string | null;
  refreshAll: () => Promise<void>;
  refreshConfigAndLogs: (containerId?: string) => Promise<void>;
  refreshServicesAndWeb: (containerId?: string) => Promise<void>;
  runLifecycle: (action: ContainerLifecycleAction) => Promise<void>;
  runMaintenance: (op: ContainerMaintenanceOp) => Promise<void>;
  updateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
  controlService: (serviceId: string, action: ServiceControlAction) => Promise<void>;
  loadServiceLogs: (serviceId: string) => Promise<void>;
  toggleWebLan: (lanEnabled: boolean) => Promise<void>;
  rotateWebToken: () => Promise<void>;
  runTask: (command: string[]) => Promise<void>;
  cancelTask: (taskId: string) => Promise<void>;
  selectTask: (taskId: string) => Promise<void>;
  clearTaskConsole: () => void;
  patchConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  addSshKey: (publicKey: string) => Promise<boolean>;
}

/**
 * Composes focused container, config, lifecycle/maintenance action, and task hooks
 * into a single runtime facade for the Strata SideVeil plugin UI.
 */
export function useStrataRuntime(isOnline: boolean, isDark: boolean): StrataRuntimeApi {
  const { bridge } = useSideVeilPlugin();
  const { pushToast } = useToast();

  const [selectedId, setSelectedIdState] = useState<string>(STRATA_PROTOCOL.DEFAULT_CONTAINER_ID);
  const [busyAction, setBusyAction] = useState<string | null>(null);

  const selectedIdRef = useRef<string>(selectedId);
  selectedIdRef.current = selectedId;

  const setSelectedId = useCallback((id: string) => {
    selectedIdRef.current = id;
    setSelectedIdState(id);
  }, []);

  const notify = useCallback<StrataNotifyFn>(
    (title, tone = 'info', description) => {
      const message = description ? `${title}：${description}` : title;
      pushToast(message, tone);
      if (bridge.env === 'host') {
        bridge.toast(message);
      }
    },
    [bridge, pushToast],
  );

  const refreshAllRef = useRef<() => Promise<void>>(async () => {});
  const stableRefreshAll = useCallback(() => refreshAllRef.current(), []);

  const taskCtrl = useStrataTasks(selectedId, notify);
  const configCtrl = useStrataConfig({
    isOnline,
    selectedIdRef,
    setBusyAction,
    notify,
    refreshAll: stableRefreshAll,
  });
  const svcWebCtrl = useStrataServicesAndWeb({
    isOnline,
    selectedIdRef,
    setBusyAction,
    notify,
  });
  const { refreshConfigAndLogs } = configCtrl;
  const { refreshServicesAndWeb } = svcWebCtrl;
  const handleRefreshPerContainer = useCallback(
    async (containerId?: string) => {
      await Promise.all([refreshConfigAndLogs(containerId), refreshServicesAndWeb(containerId)]);
    },
    [refreshConfigAndLogs, refreshServicesAndWeb],
  );
  const containerCtrl = useStrataContainers({
    isOnline,
    isDark,
    selectedId,
    setSelectedId,
    selectedIdRef,
    notify,
    onTasksLoaded: taskCtrl.replaceTasks,
    onRefreshConfigAndLogs: handleRefreshPerContainer,
  });
  refreshAllRef.current = containerCtrl.refreshAll;

  const actionCtrl = useStrataActions({
    selectedIdRef,
    setBusyAction,
    notify,
    refreshAll: containerCtrl.refreshAll,
  });

  return {
    containers: containerCtrl.containers,
    selectedId,
    setSelectedId,
    activeContainer: containerCtrl.activeContainer,
    daemonSummary: containerCtrl.daemonSummary,
    hudState: containerCtrl.hudState,
    busyAction: busyAction ?? (taskCtrl.runningTaskAction ? 'task' : null),
    loading: containerCtrl.loading,
    lastError: containerCtrl.lastError,
    tasks: taskCtrl.tasks,
    activeTaskId: taskCtrl.activeTaskId,
    taskEntries: taskCtrl.taskEntries,
    dropbearLogs: configCtrl.dropbearLogs,
    containerConfig: configCtrl.containerConfig,
    sshKeys: configCtrl.sshKeys,
    services: svcWebCtrl.services,
    serviceLogs: svcWebCtrl.serviceLogs,
    webStatus: svcWebCtrl.webStatus,
    pairingToken: svcWebCtrl.pairingToken,
    refreshAll: containerCtrl.refreshAll,
    refreshConfigAndLogs: handleRefreshPerContainer,
    refreshServicesAndWeb: svcWebCtrl.refreshServicesAndWeb,
    runLifecycle: actionCtrl.runLifecycle,
    runMaintenance: actionCtrl.runMaintenance,
    updateHud: containerCtrl.updateHud,
    controlService: svcWebCtrl.controlService,
    loadServiceLogs: svcWebCtrl.loadServiceLogs,
    toggleWebLan: svcWebCtrl.toggleWebLan,
    rotateWebToken: svcWebCtrl.rotateWebToken,
    runTask: taskCtrl.runTask,
    cancelTask: taskCtrl.cancelTask,
    selectTask: taskCtrl.selectTask,
    clearTaskConsole: taskCtrl.clearTaskConsole,
    patchConfig: configCtrl.patchConfig,
    addSshKey: configCtrl.addSshKey,
  };
}
