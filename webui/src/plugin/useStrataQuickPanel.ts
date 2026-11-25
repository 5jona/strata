import { useHaptics, useSideVeilPlugin } from '@gojona/sideveil-sdk';
import { useToast } from '@gojona/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ServiceControlAction } from '../shared/ServicesPanel.tsx';
import { formatMaintenanceToast } from '../shared/strataFormat.ts';
import {
  type ContainerServiceRuntime,
  type ContainerStatusSnapshot,
  type HudStateResult,
  STRATA_EVENTS,
  STRATA_METHODS,
  STRATA_PROTOCOL,
} from '../types.ts';
import { invokeStrataRpc, useStrataEvent } from './strataRpc.ts';

/** Supported container lifecycle commands in the SideVeil quick panel. */
export type QuickLifecycleAction = 'start' | 'stop' | 'restart';

/** Lean runtime controller backing the zero-tab, zero-IME SideVeil quick control board. */
export function useStrataQuickPanel(isOnline: boolean, isDark: boolean) {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [selectedId, setSelectedIdState] = useState<string>(STRATA_PROTOCOL.DEFAULT_CONTAINER_ID);
  const selectedIdRef = useRef(selectedId);
  selectedIdRef.current = selectedId;

  const [hudState, setHudState] = useState<HudStateResult>({
    enabled: false,
    clickThrough: false,
    isDark: true,
    sideveilConnected: false,
  });
  const [services, setServices] = useState<ContainerServiceRuntime[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  const notify = useCallback(
    (title: string, tone: 'info' | 'success' | 'danger' = 'info', description?: string) => {
      const message = description ? `${title}：${description}` : title;
      pushToast(message, tone);
      if (bridge.env === 'host') {
        bridge.toast(message);
      }
    },
    [bridge, pushToast],
  );

  const refreshServices = useCallback(
    async (containerId: string) => {
      if (!isOnline) return;
      try {
        const list = await invokeStrataRpc(bridge, STRATA_METHODS.SERVICE_LIST, {
          container: containerId,
        });
        if (selectedIdRef.current === containerId) {
          setServices(list);
        }
      } catch {
        // Keep last known service state when container transitions.
      }
    },
    [bridge, isOnline],
  );

  const refreshAll = useCallback(async () => {
    if (!isOnline) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setLastError(null);
    try {
      const [list, hud] = await Promise.all([
        invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_LIST),
        invokeStrataRpc(bridge, STRATA_METHODS.HUD_STATE),
      ]);
      setContainers(list);
      setHudState(hud);
      const nextId =
        list.find((c) => c.id === selectedIdRef.current)?.id ??
        list.find((c) => c.isDefault)?.id ??
        list[0]?.id ??
        STRATA_PROTOCOL.DEFAULT_CONTAINER_ID;
      selectedIdRef.current = nextId;
      setSelectedIdState(nextId);
      await refreshServices(nextId);
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, isOnline, refreshServices]);

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
    setContainers((prev) =>
      prev.some((c) => c.id === payload.snapshot.id)
        ? prev.map((c) => (c.id === payload.snapshot.id ? payload.snapshot : c))
        : [...prev, payload.snapshot],
    );
  });

  useStrataEvent(STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE, (payload) => {
    if (!payload) return;
    const toast = formatMaintenanceToast(payload);
    notify(toast.title, toast.tone, toast.description);
    void refreshAll();
  });

  useStrataEvent(STRATA_EVENTS.SERVICE_STATE_CHANGED, (payload) => {
    if (!payload || payload.container !== selectedIdRef.current) return;
    setServices((prev) =>
      prev.map((svc) =>
        svc.id === payload.serviceId
          ? {
              ...svc,
              status: payload.status,
              restartCount: payload.restartCount,
              ...(payload.pid !== undefined ? { pid: payload.pid } : { pid: undefined }),
              ...(payload.lastError !== undefined ? { lastError: payload.lastError } : {}),
            }
          : svc,
      ),
    );
  });

  const selectContainer = useCallback(
    (containerId: string) => {
      selectedIdRef.current = containerId;
      setSelectedIdState(containerId);
      void refreshServices(containerId);
    },
    [refreshServices],
  );

  const runLifecycle = useCallback(
    async (action: QuickLifecycleAction) => {
      const cid = selectedIdRef.current;
      setBusyAction(action);
      void triggerHaptic(action === 'stop' ? 'heavy' : 'click');
      try {
        if (action === 'start') {
          await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_START,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify('容器已启动', 'success', cid);
        } else if (action === 'stop') {
          const report = await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_STOP,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify('容器已停止', 'success', `清理 ${report.unmountedPaths} 挂载`);
        } else {
          await invokeStrataRpc(
            bridge,
            STRATA_METHODS.CONTAINER_RESTART,
            { container: cid },
            { timeoutMs: 25_000 },
          );
          notify('容器已重启', 'success', cid);
        }
        await refreshAll();
      } catch (err) {
        notify('操作失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, triggerHaptic],
  );

  const syncDns = useCallback(async () => {
    const cid = selectedIdRef.current;
    setBusyAction('dns');
    void triggerHaptic('click');
    try {
      await invokeStrataRpc(bridge, STRATA_METHODS.CONTAINER_SYNC_DNS, { container: cid });
      notify('DNS 与宿主机桥接已同步', 'success', cid);
    } catch (err) {
      notify('同步 DNS 失败', 'danger', err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  }, [bridge, notify, triggerHaptic]);

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

  const controlService = useCallback(
    async (serviceId: string, action: ServiceControlAction) => {
      const cid = selectedIdRef.current;
      setBusyAction(`svc:${serviceId}`);
      void triggerHaptic('click');
      try {
        const method =
          action === 'start'
            ? STRATA_METHODS.SERVICE_START
            : action === 'stop'
              ? STRATA_METHODS.SERVICE_STOP
              : STRATA_METHODS.SERVICE_RESTART;
        const next = await invokeStrataRpc(bridge, method, { container: cid, serviceId });
        setServices((prev) => prev.map((s) => (s.id === serviceId ? next : s)));
        notify(
          action === 'start' ? '服务已启动' : action === 'stop' ? '服务已停止' : '服务已重启',
          'success',
          next.name,
        );
      } catch (err) {
        notify('服务操作失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, triggerHaptic],
  );

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  return {
    containers,
    selectedId,
    selectContainer,
    activeContainer,
    hudState,
    services,
    loading,
    busyAction,
    lastError,
    refreshAll,
    runLifecycle,
    syncDns,
    updateHud,
    controlService,
  };
}
