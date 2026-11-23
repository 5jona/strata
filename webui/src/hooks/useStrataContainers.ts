import { useHaptics, useSideVeilPlugin } from '@pochita/sideveil-sdk';
import { type MutableRefObject, useCallback, useEffect, useState } from 'react';
import { formatMaintenanceToast } from '../lib/strataFormat.ts';
import { invokeStrataRpc, useStrataEvent } from '../lib/strataRpc.ts';
import {
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  type HudStateResult,
  STRATA_EVENTS,
  STRATA_METHODS,
  STRATA_PROTOCOL,
  type TaskSummary,
} from '../types.ts';

/** Toast notification helper signature shared across Strata runtime hooks. */
export type StrataNotifyFn = (
  title: string,
  tone?: 'info' | 'success' | 'danger',
  description?: string,
) => void;

/** Input dependencies for `useStrataContainers`. */
export interface UseStrataContainersOptions {
  isOnline: boolean;
  isDark: boolean;
  selectedId: string;
  setSelectedId: (id: string) => void;
  selectedIdRef: MutableRefObject<string>;
  notify: StrataNotifyFn;
  onTasksLoaded: (tasks: TaskSummary[]) => void;
  onRefreshConfigAndLogs: (containerId?: string) => Promise<void>;
}

/** Container list, daemon summary, HUD state, and realtime refresh controller. */
export interface StrataContainersApi {
  containers: ContainerStatusSnapshot[];
  activeContainer: ContainerStatusSnapshot | null;
  daemonSummary: DaemonSummarySnapshot | null;
  hudState: HudStateResult;
  loading: boolean;
  lastError: string | null;
  refreshAll: () => Promise<void>;
  updateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
}

/**
 * Manages container snapshots, daemon status, native HUD state, and event-driven updates.
 */
export function useStrataContainers({
  isOnline,
  isDark,
  selectedId,
  setSelectedId,
  selectedIdRef,
  notify,
  onTasksLoaded,
  onRefreshConfigAndLogs,
}: UseStrataContainersOptions): StrataContainersApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();

  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [daemonSummary, setDaemonSummary] = useState<DaemonSummarySnapshot | null>(null);
  const [hudState, setHudState] = useState<HudStateResult>({
    enabled: false,
    clickThrough: false,
    isDark: true,
    sideveilConnected: false,
  });
  const [loading, setLoading] = useState<boolean>(true);
  const [lastError, setLastError] = useState<string | null>(null);

  const refreshAll = useCallback(async () => {
    if (!isOnline) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setLastError(null);
    try {
      const [list, summary, hud, taskList] = await Promise.all([
        invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_LIST),
        invokeStrataRpc(bridge, STRATA_METHODS.DAEMON_STATUS),
        invokeStrataRpc(bridge, STRATA_METHODS.HUD_STATE),
        invokeStrataRpc(bridge, STRATA_METHODS.TASK_LIST),
      ]);
      setContainers(list);
      setDaemonSummary(summary);
      setHudState(hud);
      onTasksLoaded(taskList);

      const nextSelected =
        list.find((item) => item.id === selectedIdRef.current)?.id ??
        list.find((item) => item.isDefault)?.id ??
        list[0]?.id ??
        STRATA_PROTOCOL.DEFAULT_CONTAINER_ID;
      selectedIdRef.current = nextSelected;
      setSelectedId(nextSelected);
      await onRefreshConfigAndLogs(nextSelected);
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, isOnline, onRefreshConfigAndLogs, onTasksLoaded, selectedIdRef, setSelectedId]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    if (!isOnline) return;
    void invokeStrataRpc(bridge, STRATA_METHODS.HUD_SET, { isDark })
      .then(setHudState)
      .catch(() => {});
  }, [bridge, isDark, isOnline]);

  useStrataEvent(STRATA_EVENTS.CONTAINER_STATE_CHANGED, (payload) => {
    if (!payload?.snapshot) return;
    setContainers((prev) => {
      const exists = prev.some((c) => c.id === payload.snapshot.id);
      return exists
        ? prev.map((c) => (c.id === payload.snapshot.id ? payload.snapshot : c))
        : [...prev, payload.snapshot];
    });
  });

  useStrataEvent(STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE, (payload) => {
    if (!payload) return;
    const toast = formatMaintenanceToast(payload);
    notify(toast.title, toast.tone, toast.description);
    void refreshAll();
  });

  const updateHud = useCallback(
    async (patch: { enabled?: boolean; clickThrough?: boolean }) => {
      void triggerHaptic('tick');
      try {
        const next = await invokeStrataRpc(bridge, STRATA_METHODS.HUD_SET, { ...patch, isDark });
        setHudState(next);
      } catch (err) {
        notify('HUD 更新失败', 'danger', err instanceof Error ? err.message : String(err));
      }
    },
    [bridge, isDark, notify, triggerHaptic],
  );

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  return {
    containers,
    activeContainer,
    daemonSummary,
    hudState,
    loading,
    lastError,
    refreshAll,
    updateHud,
  };
}
