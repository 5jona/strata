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

export interface CreateContainerOptions {
  sshPort?: number;
  imageSizeGb?: number;
  setDefault?: boolean;
}

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
    let spec = containerConfig?.services.find((s) => s.id === serviceId);
    if (!spec) {
      try {
        const cfgRes = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_GET, {
          container: activeContainer.id,
        });
        setContainerConfig(cfgRes.config);
        spec = cfgRes.config.services.find((s) => s.id === serviceId);
      } catch (err) {
        pushToast(errMsg(err), 'danger');
        return;
      }
    }
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

  const setDefaultContainer = async (containerId: string) => {
    setBusyKey(`container:default:${containerId}`);
    try {
      await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SET_DEFAULT, { id: containerId });
      pushToast(`已将 ${containerId} 设为默认容器`, 'success');
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const deleteContainer = async (containerId: string, deleteImage: boolean) => {
    setBusyKey(`container:delete:${containerId}`);
    try {
      await invokeStrataKsuRpc(
        bridge,
        STRATA_METHODS.CONTAINER_DELETE,
        { id: containerId, deleteImage, force: true },
        30_000,
      );
      pushToast(
        deleteImage ? `已删除容器 ${containerId} 及磁盘镜像` : `已移除容器 ${containerId} 配置`,
        'success',
      );
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const createContainer = async (containerId: string, options?: CreateContainerOptions) => {
    setBusyKey(`container:create:${containerId}`);
    try {
      const sizeMb = Math.round((options?.imageSizeGb ?? 4) * 1024);
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CREATE, {
        id: containerId,
        distroId: 'debian-12',
        sizeMb,
        enableSsh: true,
        ...(options?.sshPort !== undefined ? { sshPort: options.sshPort } : {}),
        autoStart: true,
        setDefault: options?.setDefault ?? activeContainer === null,
        hostBridge: true,
        l5DiagnosticMounts: false,
        syncAndroidDns: true,
        noatime: true,
        sshPublicKeys: [],
      });
      pushToast(`已提交创建任务 ${res.jobId}，后台正在拉取镜像`, 'info');
      await refreshAll();
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
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
    setDefaultContainer,
    deleteContainer,
    createContainer,
    startDaemon,
  };
}
