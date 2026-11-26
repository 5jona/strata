import type { BridgeClient } from '@gojona/bridge';
import type { Dispatch, SetStateAction } from 'react';
import type { ServiceControlAction } from '../shared/ServicesPanel.tsx';
import { formatBytes } from '../shared/strataFormat.ts';
import {
  type ContainerConfig,
  type ContainerServiceRuntime,
  type ContainerStatusSnapshot,
  STRATA_METHODS,
} from '../types.ts';
import { invokeStrataKsuRpc, startStrataDaemonViaKsu } from './ksuStrataClient.ts';

export type KsuMaintenanceOp = 'dns' | 'compact' | 'fsck' | 'backup';

export interface UseKsuActionsOptions {
  bridge: BridgeClient;
  activeContainer: ContainerStatusSnapshot | null;
  containerConfig: ContainerConfig | null;
  setBusyKey: (key: string | null) => void;
  setServices: Dispatch<SetStateAction<ContainerServiceRuntime[]>>;
  setContainerConfig: Dispatch<SetStateAction<ContainerConfig | null>>;
  setServiceLogs: Dispatch<SetStateAction<Record<string, string[]>>>;
  refreshAll: () => Promise<void>;
  pushToast: (message: string, tone?: 'info' | 'success' | 'danger') => void;
}

/** Container lifecycle, maintenance, service supervision, and daemon rescue actions for KSU. */
export function useKsuActions({
  bridge,
  activeContainer,
  containerConfig,
  setBusyKey,
  setServices,
  setContainerConfig,
  setServiceLogs,
  refreshAll,
  pushToast,
}: UseKsuActionsOptions) {
  const errMsg = (err: unknown) => (err instanceof Error ? err.message : String(err));

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
      await invokeStrataKsuRpc(bridge, method, { container: activeContainer.id }, 25_000);
      pushToast(
        action === 'start' ? '容器已启动' : action === 'stop' ? '容器已停止' : '容器已重启',
        'success',
      );
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const runMaintenance = async (op: KsuMaintenanceOp) => {
    if (!activeContainer) return;
    const cid = activeContainer.id;
    setBusyKey(`maint:${op}`);
    try {
      if (op === 'dns') {
        await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SYNC_DNS, { container: cid });
        pushToast('已同步容器 DNS 解析', 'success');
      } else if (op === 'compact') {
        const res = await invokeStrataKsuRpc(
          bridge,
          STRATA_METHODS.CONTAINER_COMPACT,
          { container: cid },
          45_000,
        );
        pushToast(
          `空闲空间回收完成：${formatBytes(res.beforeBytes)} → ${formatBytes(res.afterBytes)}`,
          'success',
        );
      } else if (op === 'fsck') {
        const res = await invokeStrataKsuRpc(
          bridge,
          STRATA_METHODS.CONTAINER_FSCK,
          { container: cid },
          45_000,
        );
        pushToast(
          res.exitCode <= 1
            ? '磁盘检查完成，文件系统正常'
            : `磁盘检查异常（退出码 ${res.exitCode}）`,
          res.exitCode <= 1 ? 'success' : 'danger',
        );
      } else {
        const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_BACKUP_START, {
          container: cid,
        });
        pushToast(`后台备份已启动：${res.outputPath}`, 'info');
      }
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
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
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const toggleServiceAutoStart = async (serviceId: string, autoStart: boolean) => {
    if (!activeContainer) return;
    const spec = containerConfig?.services.find((s) => s.id === serviceId);
    if (!spec) {
      pushToast('未找到服务配置定义，请先刷新配置', 'danger');
      return;
    }
    setBusyKey(`svc-auto:${serviceId}`);
    try {
      const updatedSpec = { ...spec, autoStart };
      const next = await invokeStrataKsuRpc(bridge, STRATA_METHODS.SERVICE_UPSERT, {
        container: activeContainer.id,
        spec: updatedSpec,
      });
      setServices((prev) => prev.map((s) => (s.id === serviceId ? next : s)));
      setContainerConfig((prev) =>
        prev
          ? { ...prev, services: prev.services.map((s) => (s.id === serviceId ? updatedSpec : s)) }
          : prev,
      );
      pushToast(`${next.name} 已${autoStart ? '开启' : '关闭'}开机自启`, 'success');
    } catch (err) {
      pushToast(errMsg(err), 'danger');
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
      pushToast(errMsg(err), 'danger');
    }
  };

  const startDaemon = async () => {
    setBusyKey('daemon:rescue');
    try {
      await startStrataDaemonViaKsu(bridge);
      pushToast('正在启动 Strata 后台服务，请稍候…', 'info');
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  return {
    runLifecycle,
    runMaintenance,
    runServiceAction,
    toggleServiceAutoStart,
    loadServiceLogs,
    startDaemon,
  };
}
