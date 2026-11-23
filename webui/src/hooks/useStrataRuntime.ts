import { useSideVeilPlugin } from '@gojona/sideveil-sdk';
import { type LogEntry, useToast } from '@gojona/ui';
import { useCallback, useRef, useState } from 'react';
import {
  type ContainerConfig,
  type ContainerConfigPatchParams,
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  type HudStateResult,
  STRATA_PROTOCOL,
  type TaskSummary,
} from '../types.ts';
import {
  type ContainerLifecycleAction,
  type ContainerMaintenanceOp,
  useStrataActions,
} from './useStrataActions.ts';
import { useStrataConfig } from './useStrataConfig.ts';
import { type StrataNotifyFn, useStrataContainers } from './useStrataContainers.ts';
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
  refreshAll: () => Promise<void>;
  refreshConfigAndLogs: (containerId?: string) => Promise<void>;
  runLifecycle: (action: ContainerLifecycleAction) => Promise<void>;
  runMaintenance: (op: ContainerMaintenanceOp) => Promise<void>;
  updateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
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
  const containerCtrl = useStrataContainers({
    isOnline,
    isDark,
    selectedId,
    setSelectedId,
    selectedIdRef,
    notify,
    onTasksLoaded: taskCtrl.replaceTasks,
    onRefreshConfigAndLogs: configCtrl.refreshConfigAndLogs,
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
    refreshAll: containerCtrl.refreshAll,
    refreshConfigAndLogs: configCtrl.refreshConfigAndLogs,
    runLifecycle: actionCtrl.runLifecycle,
    runMaintenance: actionCtrl.runMaintenance,
    updateHud: containerCtrl.updateHud,
    runTask: taskCtrl.runTask,
    cancelTask: taskCtrl.cancelTask,
    selectTask: taskCtrl.selectTask,
    clearTaskConsole: taskCtrl.clearTaskConsole,
    patchConfig: configCtrl.patchConfig,
    addSshKey: configCtrl.addSshKey,
  };
}
