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

export const KSU_TABS = ['overview', 'services', 'config', 'diagnostics', 'settings'] as const;
export type KsuTab = (typeof KSU_TABS)[number];

/** Type guard for KernelSU workspace tab identifiers. */
export function isKsuTab(value: string): value is KsuTab {
  return (
    value === 'overview' ||
    value === 'services' ||
    value === 'config' ||
    value === 'diagnostics' ||
    value === 'settings'
  );
}

/** Runtime state and lazy per-tab RPC handlers backing the standalone KernelSU WebUI. */
export function useKsuStrataRuntime() {
  const bridge = useBridge();
  const { pushToast } = useToast();

  const [activeTab, setActiveTabState] = useState<KsuTab>('overview');
  const activeTabRef = useRef<KsuTab>('overview');
  const loadedTabsRef = useRef<Set<string>>(new Set());

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

  const refreshContainerServices = useCallback(
    async (containerId: string) => {
      const svcList = await invokeStrataKsuRpc(bridge, STRATA_METHODS.SERVICE_LIST, {
        container: containerId,
      });
      if (selectedIdRef.current === containerId) {
        setServices(svcList);
      }
    },
    [bridge],
  );

  const refreshContainerConfig = useCallback(
    async (containerId: string) => {
      const [cfgRes, keysRes] = await Promise.allSettled([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_GET, { container: containerId }),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_LIST, {
          container: containerId,
        }),
      ]);
      if (selectedIdRef.current !== containerId) return;
      if (cfgRes.status === 'fulfilled') setContainerConfig(cfgRes.value.config);
      if (keysRes.status === 'fulfilled') setSshKeys(keysRes.value.keys);
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

  const refreshWebStatus = useCallback(async () => {
    const web = await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_STATUS);
    setWebStatus(web);
  }, [bridge]);

  const ensureTabLoaded = useCallback(
    async (tab: KsuTab, containerId: string | null, force = false) => {
      const key = `${tab}:${tab === 'settings' ? 'global' : (containerId ?? '')}`;
      if (!force && loadedTabsRef.current.has(key)) return;
      loadedTabsRef.current.add(key);
      try {
        if (tab === 'services' && containerId) {
          await refreshContainerServices(containerId);
        } else if (tab === 'config' && containerId) {
          await refreshContainerConfig(containerId);
        } else if (tab === 'diagnostics') {
          await refreshDiagnostics(containerId);
        } else if (tab === 'settings') {
          await refreshWebStatus();
        }
      } catch {
        loadedTabsRef.current.delete(key);
      }
    },
    [refreshContainerConfig, refreshContainerServices, refreshDiagnostics, refreshWebStatus],
  );

  const refreshAll = useCallback(async () => {
    setLoading(true);
    setLastError(null);
    loadedTabsRef.current.clear();
    try {
      const [summary, list] = await Promise.all([
        invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_STATUS),
        invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_LIST),
      ]);
      setDaemon(summary);
      setContainers(list);

      const nextId =
        list.find((c) => c.id === selectedIdRef.current)?.id ??
        list.find((c) => c.isDefault)?.id ??
        list[0]?.id ??
        null;
      selectedIdRef.current = nextId;
      setSelectedId(nextId);

      if (!nextId) {
        setServices([]);
        setContainerConfig(null);
        setSshKeys([]);
        setDropbearLogs([]);
      }
      if (activeTabRef.current !== 'overview') {
        await ensureTabLoaded(activeTabRef.current, nextId, true);
      }
    } catch (err) {
      setDaemon(null);
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, ensureTabLoaded]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  const setActiveTab = useCallback(
    (nextTab: KsuTab) => {
      activeTabRef.current = nextTab;
      setActiveTabState(nextTab);
      void ensureTabLoaded(nextTab, selectedIdRef.current);
    },
    [ensureTabLoaded],
  );

  const selectContainer = useCallback(
    (containerId: string) => {
      selectedIdRef.current = containerId;
      setSelectedId(containerId);
      setServices([]);
      setContainerConfig(null);
      setSshKeys([]);
      setDropbearLogs([]);
      if (activeTabRef.current !== 'overview' && activeTabRef.current !== 'settings') {
        void ensureTabLoaded(activeTabRef.current, containerId, true);
      }
    },
    [ensureTabLoaded],
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
    activeTab,
    setActiveTab,
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
    refreshContainerDetails: refreshContainerServices,
    refreshDiagnostics,
    copyWithFeedback,
    copyRawText,
    ...actions,
    ...configActions,
  };
}

export type KsuStrataRuntimeApi = ReturnType<typeof useKsuStrataRuntime>;
