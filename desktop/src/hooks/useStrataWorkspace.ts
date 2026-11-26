import { useToast } from '@gojona/ui';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatMaintenanceOpLabel } from '../lib/format.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type {
  ContainerConfig,
  ContainerJobProgressPayload,
  ContainerServiceRuntime,
  ContainerStatusSnapshot,
  DaemonSummarySnapshot,
  KernelCapabilitiesResult,
  WebStatusResult,
} from '../types/protocol.ts';
import { STRATA_EVENTS, STRATA_METHODS } from '../types/rpc.ts';

export interface StrataWorkspaceState {
  loading: boolean;
  daemonStatus: DaemonSummarySnapshot | null;
  containers: ContainerStatusSnapshot[];
  selectedContainerId: string | undefined;
  setSelectedContainerId: (id: string | undefined) => void;
  capabilities: KernelCapabilitiesResult | null;
  webStatus: WebStatusResult | null;
  servicesByContainer: Record<string, ContainerServiceRuntime[]>;
  configsByContainer: Record<string, ContainerConfig>;
  activeJobs: ContainerJobProgressPayload[];
  dismissJob: (jobId: string) => void;
  refreshAll: () => Promise<void>;
  refreshServices: (containerId: string) => Promise<void>;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Synchronizes active device containers, global selected container context, daemon summary,
 * kernel capabilities, managed services, container configs, and live WebSocket push events.
 */
export function useStrataWorkspace(transport: StrataTransport | null): StrataWorkspaceState {
  const { pushToast } = useToast();
  const [loading, setLoading] = useState(false);
  const [daemonStatus, setDaemonStatus] = useState<DaemonSummarySnapshot | null>(null);
  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [selectedId, setSelectedId] = useState<string | undefined>(undefined);
  const [capabilities, setCapabilities] = useState<KernelCapabilitiesResult | null>(null);
  const [webStatus, setWebStatus] = useState<WebStatusResult | null>(null);
  const [servicesByContainer, setServicesByContainer] = useState<
    Record<string, ContainerServiceRuntime[]>
  >({});
  const [configsByContainer, setConfigsByContainer] = useState<Record<string, ContainerConfig>>({});
  const [activeJobs, setActiveJobs] = useState<ContainerJobProgressPayload[]>([]);

  const selectedContainerId = useMemo(() => {
    if (containers.length === 0) return undefined;
    if (selectedId && containers.some((c) => c.id === selectedId)) {
      return selectedId;
    }
    const defaultId = daemonStatus?.defaultContainer;
    if (defaultId && containers.some((c) => c.id === defaultId)) {
      return defaultId;
    }
    return containers.find((c) => c.isDefault)?.id ?? containers[0]?.id;
  }, [containers, daemonStatus?.defaultContainer, selectedId]);

  const refreshServices = useCallback(
    async (containerId: string) => {
      if (!transport) return;
      try {
        const list = await transport.invoke(STRATA_METHODS.SERVICE_LIST, {
          container: containerId,
        });
        setServicesByContainer((prev) => ({ ...prev, [containerId]: list }));
      } catch {
        // Ignore transient service query failure when container is being provisioned
      }
    },
    [transport],
  );

  const refreshAll = useCallback(async () => {
    if (!transport) {
      setDaemonStatus(null);
      setContainers([]);
      setSelectedId(undefined);
      setCapabilities(null);
      setWebStatus(null);
      setServicesByContainer({});
      setConfigsByContainer({});
      return;
    }
    setLoading(true);
    try {
      const [status, list, caps, web] = await Promise.all([
        transport.invoke(STRATA_METHODS.DAEMON_STATUS),
        transport.invoke(STRATA_METHODS.CONTAINER_LIST),
        transport.invoke(STRATA_METHODS.DAEMON_CAPABILITIES),
        transport.invoke(STRATA_METHODS.WEB_STATUS),
      ]);
      setDaemonStatus(status);
      setContainers(list);
      setCapabilities(caps);
      setWebStatus(web);

      const detailEntries = await Promise.all(
        list.map(async (c) => {
          const [svcsRes, cfgRes] = await Promise.allSettled([
            transport.invoke(STRATA_METHODS.SERVICE_LIST, { container: c.id }),
            transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_GET, { container: c.id }),
          ]);
          return {
            id: c.id,
            svcs:
              svcsRes.status === 'fulfilled' ? svcsRes.value : ([] as ContainerServiceRuntime[]),
            cfg: cfgRes.status === 'fulfilled' ? cfgRes.value.config : null,
          };
        }),
      );
      setServicesByContainer(Object.fromEntries(detailEntries.map((item) => [item.id, item.svcs])));
      setConfigsByContainer(
        Object.fromEntries(
          detailEntries.filter((item) => item.cfg !== null).map((item) => [item.id, item.cfg]),
        ) as Record<string, ContainerConfig>,
      );
    } catch (err) {
      pushToast(`刷新设备状态失败：${err instanceof Error ? err.message : String(err)}`, 'danger');
    } finally {
      setLoading(false);
    }
  }, [pushToast, transport]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    if (!transport) return;
    return transport.subscribe((envelope) => {
      if (envelope.event === STRATA_EVENTS.CONTAINER_STATE_CHANGED) {
        const snap = envelope.payload.snapshot;
        setContainers((prev) => {
          const exists = prev.some((c) => c.id === snap.id);
          return exists ? prev.map((c) => (c.id === snap.id ? snap : c)) : [...prev, snap];
        });
        void refreshServices(snap.id);
      } else if (envelope.event === STRATA_EVENTS.SERVICE_STATE_CHANGED) {
        const { container, serviceId, status, pid, restartCount, lastError } = envelope.payload;
        setServicesByContainer((prev) => {
          const existing = prev[container] ?? [];
          return {
            ...prev,
            [container]: existing.map((svc) =>
              svc.id === serviceId
                ? {
                    ...svc,
                    status,
                    restartCount,
                    ...(pid !== undefined ? { pid } : { pid: undefined }),
                    ...(lastError !== undefined ? { lastError } : {}),
                  }
                : svc,
            ),
          };
        });
      } else if (envelope.event === STRATA_EVENTS.CONTAINER_JOB_PROGRESS) {
        const job = envelope.payload;
        setActiveJobs((prev) => {
          const idx = prev.findIndex((j) => j.jobId === job.jobId);
          if (idx >= 0) {
            const copy = [...prev];
            copy[idx] = job;
            return copy;
          }
          return [job, ...prev].slice(0, 8);
        });
        if (job.state === 'completed') {
          pushToast(`容器 ${job.container} 准备完成`, 'success');
          void refreshAll();
        } else if (job.state === 'failed') {
          pushToast(`容器 ${job.container} 任务失败：${job.error ?? '未知错误'}`, 'danger');
        }
      } else if (envelope.event === STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE) {
        const { container, op, ok, summary, error } = envelope.payload;
        const opLabel = formatMaintenanceOpLabel(op);
        pushToast(
          ok
            ? `${container} ${opLabel}完成：${summary}`
            : `${container} ${opLabel}失败：${error ?? summary}`,
          ok ? 'success' : 'danger',
        );
        void refreshAll();
      }
    });
  }, [pushToast, refreshAll, refreshServices, transport]);

  const dismissJob = useCallback((jobId: string) => {
    setActiveJobs((prev) => prev.filter((j) => j.jobId !== jobId));
  }, []);

  const runAction = useCallback(
    async <T>(label: string, fn: () => Promise<T>): Promise<T | null> => {
      try {
        const res = await fn();
        return res;
      } catch (err) {
        pushToast(`${label}失败：${err instanceof Error ? err.message : String(err)}`, 'danger');
        return null;
      }
    },
    [pushToast],
  );

  return {
    loading,
    daemonStatus,
    containers,
    selectedContainerId,
    setSelectedContainerId: setSelectedId,
    capabilities,
    webStatus,
    servicesByContainer,
    configsByContainer,
    activeJobs,
    dismissJob,
    refreshAll,
    refreshServices,
    runAction,
  };
}
