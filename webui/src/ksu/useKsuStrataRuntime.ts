import { useBridge } from '@gojona/bridge';
import { copyTextWithFallback, type LogEntry, useToast } from '@gojona/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { toLogEntries } from '../shared/strataFormat.ts';
import {
  type ContainerConfig,
  type ContainerServiceRuntime,
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  type KernelCapabilitiesResult,
  STRATA_METHODS,
  type WebStatusResult,
} from '../types.ts';
import { invokeStrataKsuRpc } from './ksuStrataClient.ts';
import { useKsuActions } from './useKsuActions.ts';
import { useKsuConfigActions } from './useKsuConfigActions.ts';

/** Runtime state and RPC command handlers backing the standalone KernelSU WebUI. */
export function useKsuStrataRuntime() {
  const bridge = useBridge();
  const { pushToast } = useToast();

  const [loading, setLoading] = useState(true);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [daemon, setDaemon] = useState<DaemonSummarySnapshot | null>(null);
  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selectedIdRef = useRef<string | null>(selectedId);
  selectedIdRef.current = selectedId;

  const [services, setServices] = useState<ContainerServiceRuntime[]>([]);
  const [serviceLogs, setServiceLogs] = useState<Record<string, string[]>>({});
  const [containerConfig, setContainerConfig] = useState<ContainerConfig | null>(null);
  const [sshKeys, setSshKeys] = useState<string[]>([]);
  const [dropbearLogs, setDropbearLogs] = useState<LogEntry[]>([]);
  const [daemonLogs, setDaemonLogs] = useState<LogEntry[]>([]);
  const [capabilities, setCapabilities] = useState<KernelCapabilitiesResult | null>(null);
  const [webStatus, setWebStatus] = useState<WebStatusResult | null>(null);
  const [pairingToken, setPairingToken] = useState<string | null>(null);

  const refreshContainerDetails = useCallback(
    async (containerId: string) => {
      const [svcRes, cfgRes, keysRes, logsRes] = await Promise.allSettled([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.SERVICE_LIST, { container: containerId }),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_GET, { container: containerId }),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_LIST, {
          container: containerId,
        }),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_LOGS, {
          container: containerId,
          lines: 120,
        }),
      ]);
      if (selectedIdRef.current !== containerId) return;
      if (svcRes.status === 'fulfilled') setServices(svcRes.value);
      if (cfgRes.status === 'fulfilled') setContainerConfig(cfgRes.value.config);
      if (keysRes.status === 'fulfilled') setSshKeys(keysRes.value.keys);
      if (logsRes.status === 'fulfilled') setDropbearLogs(toLogEntries(logsRes.value.lines, 'db'));
    },
    [bridge],
  );

  const refreshDiagnostics = useCallback(
    async (containerId?: string | null) => {
      const cid = containerId ?? selectedIdRef.current;
      const [dLogsRes, cLogsRes, capsRes] = await Promise.allSettled([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_LOGS, { lines: 120 }),
        cid
          ? invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_LOGS, {
              container: cid,
              lines: 120,
            })
          : Promise.resolve({ lines: [] as string[] }),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_CAPABILITIES),
      ]);
      if (dLogsRes.status === 'fulfilled')
        setDaemonLogs(toLogEntries(dLogsRes.value.lines, 'daemon'));
      if (cLogsRes.status === 'fulfilled')
        setDropbearLogs(toLogEntries(cLogsRes.value.lines, 'db'));
      if (capsRes.status === 'fulfilled') setCapabilities(capsRes.value);
    },
    [bridge],
  );

  const refreshAll = useCallback(async () => {
    setLoading(true);
    setLastError(null);
    try {
      const [summary, list, web, capsRes, dLogsRes] = await Promise.all([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_STATUS),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_LIST),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_STATUS),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_CAPABILITIES).catch(() => null),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_LOGS, { lines: 120 }).catch(() => null),
      ]);
      setDaemon(summary);
      setContainers(list);
      setWebStatus(web);
      if (capsRes) setCapabilities(capsRes);
      if (dLogsRes) setDaemonLogs(toLogEntries(dLogsRes.lines, 'daemon'));

      const nextId =
        list.find((c) => c.id === selectedIdRef.current)?.id ??
        list.find((c) => c.isDefault)?.id ??
        list[0]?.id ??
        null;
      selectedIdRef.current = nextId;
      setSelectedId(nextId);
      if (nextId) {
        await refreshContainerDetails(nextId);
      } else {
        setServices([]);
        setContainerConfig(null);
        setSshKeys([]);
        setDropbearLogs([]);
      }
    } catch (err) {
      setDaemon(null);
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, refreshContainerDetails]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  const selectContainer = useCallback(
    (containerId: string) => {
      selectedIdRef.current = containerId;
      setSelectedId(containerId);
      void refreshContainerDetails(containerId);
    },
    [refreshContainerDetails],
  );

  const copyWithFeedback = useCallback(
    async (text: string, label: string) => {
      const ok = await copyTextWithFallback(text);
      pushToast(ok ? `已复制 ${label}` : `复制 ${label} 失败`, ok ? 'success' : 'danger');
    },
    [pushToast],
  );

  const copyRawText = useCallback(
    async (text: string): Promise<boolean> => {
      const ok = await copyTextWithFallback(text);
      pushToast(ok ? '已复制日志到剪贴板' : '复制日志失败', ok ? 'success' : 'danger');
      return ok;
    },
    [pushToast],
  );

  const actions = useKsuActions({
    bridge,
    activeContainer,
    containerConfig,
    setBusyKey,
    setServices,
    setContainerConfig,
    setServiceLogs,
    refreshAll,
    pushToast,
  });

  const configActions = useKsuConfigActions({
    bridge,
    activeContainer,
    setBusyKey,
    setContainerConfig,
    setSshKeys,
    setWebStatus,
    setPairingToken,
    refreshAll,
    pushToast,
  });

  return {
    loading,
    busyKey,
    lastError,
    daemon,
    containers,
    selectedId,
    selectContainer,
    activeContainer,
    services,
    serviceLogs,
    containerConfig,
    sshKeys,
    dropbearLogs,
    daemonLogs,
    capabilities,
    webStatus,
    pairingToken,
    refreshAll,
    refreshContainerDetails,
    refreshDiagnostics,
    copyWithFeedback,
    copyRawText,
    ...actions,
    ...configActions,
  };
}

export type KsuStrataRuntimeApi = ReturnType<typeof useKsuStrataRuntime>;
