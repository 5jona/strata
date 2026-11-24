import { useBridge } from '@gojona/bridge';
import { copyTextWithFallback, useToast } from '@gojona/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ServiceControlAction } from '../components/ServicesPanel.tsx';
import {
  type ContainerServiceRuntime,
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  STRATA_METHODS,
  STRATA_PROTOCOL,
  type WebStatusResult,
} from '../types.ts';
import { invokeStrataKsuRpc, readStrataKsuWebToken } from './ksuStrataClient.ts';

/** Runtime state and RPC command handlers backing the standalone KernelSU WebUI. */
export function useKsuStrataRuntime() {
  const bridge = useBridge();
  const { pushToast } = useToast();

  const [loading, setLoading] = useState(true);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [daemon, setDaemon] = useState<DaemonSummarySnapshot | null>(null);
  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [selectedId, setSelectedId] = useState<string>(STRATA_PROTOCOL.DEFAULT_CONTAINER_ID);
  const selectedIdRef = useRef<string>(selectedId);
  selectedIdRef.current = selectedId;
  const [services, setServices] = useState<ContainerServiceRuntime[]>([]);
  const [serviceLogs, setServiceLogs] = useState<Record<string, string[]>>({});
  const [webStatus, setWebStatus] = useState<WebStatusResult | null>(null);
  const [pairingToken, setPairingToken] = useState<string | null>(null);

  const refreshServices = useCallback(
    async (containerId: string) => {
      try {
        const list = await invokeStrataKsuRpc(bridge, STRATA_METHODS.SERVICE_LIST, {
          container: containerId,
        });
        setServices(list);
      } catch {
        setServices([]);
      }
    },
    [bridge],
  );

  const refreshAll = useCallback(async () => {
    setLoading(true);
    setLastError(null);
    try {
      const [summary, list, web] = await Promise.all([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_STATUS),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_LIST),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_STATUS),
      ]);
      setDaemon(summary);
      setContainers(list);
      setWebStatus(web);
      const nextId =
        list.find((c) => c.id === selectedIdRef.current)?.id ??
        list.find((c) => c.isDefault)?.id ??
        list[0]?.id ??
        STRATA_PROTOCOL.DEFAULT_CONTAINER_ID;
      selectedIdRef.current = nextId;
      setSelectedId(nextId);
      await refreshServices(nextId);
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, refreshServices]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  const copyWithFeedback = useCallback(
    async (text: string, label: string) => {
      const ok = await copyTextWithFallback(text);
      pushToast(ok ? `已复制 ${label}` : `复制 ${label} 失败`, ok ? 'success' : 'danger');
    },
    [pushToast],
  );

  const runLifecycle = async (action: 'start' | 'stop' | 'restart') => {
    if (!activeContainer) return;
    setBusyKey(`container:${action}`);
    try {
      const method =
        action === 'start'
          ? STRATA_METHODS.CONTAINER_START
          : action === 'stop'
            ? STRATA_METHODS.CONTAINER_STOP
            : STRATA_METHODS.CONTAINER_RESTART;
      await invokeStrataKsuRpc(bridge, method, { container: activeContainer.id });
      pushToast(
        action === 'start' ? '容器已启动' : action === 'stop' ? '容器已停止' : '容器已重启',
        'success',
      );
      await refreshAll();
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const runServiceAction = async (serviceId: string, action: ServiceControlAction) => {
    if (!activeContainer) return;
    setBusyKey(`svc:${serviceId}`);
    try {
      const method =
        action === 'start'
          ? STRATA_METHODS.SERVICE_START
          : action === 'stop'
            ? STRATA_METHODS.SERVICE_STOP
            : STRATA_METHODS.SERVICE_RESTART;
      const next = await invokeStrataKsuRpc(bridge, method, {
        container: activeContainer.id,
        serviceId,
      });
      setServices((prev) => prev.map((s) => (s.id === serviceId ? next : s)));
      pushToast(`服务 ${next.name} 状态已更新`, 'success');
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const loadServiceLogs = async (serviceId: string) => {
    if (!activeContainer) return;
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.SERVICE_LOGS, {
        container: activeContainer.id,
        serviceId,
        limit: 80,
      });
      setServiceLogs((prev) => ({ ...prev, [serviceId]: res.lines }));
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    }
  };

  const toggleLan = async (lanEnabled: boolean) => {
    setBusyKey('web:lan');
    try {
      const next = await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_CONFIGURE, { lanEnabled });
      setWebStatus(next);
      pushToast(lanEnabled ? '已开启局域网配对' : '已切换为本机回环', 'success');
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const readToken = async () => {
    setBusyKey('web:read');
    try {
      const res = await readStrataKsuWebToken(bridge);
      setPairingToken(res.token);
      pushToast('已读取当前 Bearer Token', 'success');
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const rotateToken = async () => {
    setBusyKey('web:rotate');
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_ROTATE_TOKEN);
      setPairingToken(res.token);
      pushToast(`Token 已轮换（已吊销 ${res.sessionsRevoked} 个会话）`, 'success');
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  return {
    loading,
    busyKey,
    lastError,
    daemon,
    containers,
    selectedId,
    setSelectedId,
    activeContainer,
    services,
    serviceLogs,
    webStatus,
    pairingToken,
    refreshAll,
    refreshServices,
    copyWithFeedback,
    runLifecycle,
    runServiceAction,
    loadServiceLogs,
    toggleLan,
    readToken,
    rotateToken,
  };
}
