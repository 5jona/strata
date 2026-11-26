import { Button, ErrorBoundary, StatusBanner, ThemeProvider, ToastProvider } from '@gojona/ui';
import { lazy, Suspense, useCallback, useMemo, useState } from 'react';
import { AddDeviceModal } from './components/AddDeviceModal.tsx';
import { ContainersView } from './components/ContainersView.tsx';
import { FirstRunOnboarding } from './components/FirstRunOnboarding.tsx';
import { InstallView } from './components/InstallView.tsx';
import { LogsKernelView } from './components/LogsKernelView.tsx';
import { NetworkSshView } from './components/NetworkSshView.tsx';
import { ServicesView } from './components/ServicesView.tsx';
import { Sidebar } from './components/Sidebar.tsx';
import { ToolsView } from './components/tools/ToolsView.tsx';
import { useDesktopDevices } from './hooks/useDesktopDevices.ts';
import { useStrataWorkspace } from './hooks/useStrataWorkspace.ts';
import type { DeviceProfile, NavViewId, TerminalTabSpec } from './types/device.ts';
import { STRATA_METHODS } from './types/rpc.ts';

const LazyTerminalWorkspace = lazy(() => import('./components/TerminalWorkspace.tsx'));

function StrataDesktopShell() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [activeView, setActiveView] = useState<NavViewId>('containers');
  const [deviceModalOpen, setDeviceModalOpen] = useState(false);
  const [editingDevice, setEditingDevice] = useState<DeviceProfile | null>(null);
  const [terminalMounted, setTerminalMounted] = useState(false);
  const [tabs, setTabs] = useState<TerminalTabSpec[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  const devicesState = useDesktopDevices();
  const workspace = useStrataWorkspace(devicesState.transport);
  const activeDevice = devicesState.activeDevice;
  const activeTransport = devicesState.transport;

  const deviceId = activeDevice?.id ?? 'none';
  const deviceTabs = useMemo(() => tabs.filter((t) => t.deviceId === deviceId), [deviceId, tabs]);
  const activeDeviceTabId = useMemo(() => {
    if (activeTabId && deviceTabs.some((t) => t.id === activeTabId)) return activeTabId;
    return deviceTabs[0]?.id ?? null;
  }, [activeTabId, deviceTabs]);

  const runningServiceCount = useMemo(
    () =>
      Object.values(workspace.servicesByContainer)
        .flat()
        .filter((s) => s.status === 'running').length,
    [workspace.servicesByContainer],
  );

  const handleOpenTerminal = useCallback(
    (containerId: string) => {
      const id = `term-${deviceId}-${containerId}-${Date.now().toString(36)}`;
      const existingCount = tabs.filter(
        (t) => t.deviceId === deviceId && t.containerId === containerId,
      ).length;
      const title = existingCount === 0 ? containerId : `${containerId} #${existingCount + 1}`;
      workspace.setSelectedContainerId(containerId);
      setTabs((prev) => [...prev, { id, deviceId, containerId, title }]);
      setActiveTabId(id);
      setTerminalMounted(true);
      setActiveView('terminal');
    },
    [deviceId, tabs, workspace],
  );

  const handleCloseTab = useCallback(
    (tabId: string) => {
      const target = tabs.find((t) => t.id === tabId);
      if (target?.sessionId && activeTransport) {
        void activeTransport.killPty(target.sessionId).catch(() => {});
      }
      setTabs((prev) => prev.filter((t) => t.id !== tabId));
      if (activeTabId === tabId) setActiveTabId(null);
    },
    [activeTabId, activeTransport, tabs],
  );

  const handleBindSession = useCallback((tabId: string, sessionId: string) => {
    setTabs((prev) => {
      if (prev.some((t) => t.id === tabId && t.sessionId === sessionId)) return prev;
      return prev.map((t) => (t.id === tabId ? { ...t, sessionId } : t));
    });
  }, []);

  const handleSelectView = useCallback((view: NavViewId) => {
    if (view === 'terminal') setTerminalMounted(true);
    setActiveView(view);
  }, []);

  const handleStartContainer = useCallback(
    async (containerId: string) => {
      if (!activeTransport) return;
      await workspace.runAction(`启动 ${containerId}`, () =>
        activeTransport.invoke(STRATA_METHODS.CONTAINER_START, { container: containerId }),
      );
      await workspace.refreshAll();
    },
    [activeTransport, workspace],
  );

  const openDeviceModal = (dev: DeviceProfile | null) => {
    setEditingDevice(dev);
    setDeviceModalOpen(true);
  };

  return (
    <div className="flex h-full w-full overflow-hidden bg-[var(--surface-canvas)] text-[var(--text-primary)]">
      <Sidebar
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        activeView={activeView}
        onSelectView={handleSelectView}
        devices={devicesState.devices}
        activeDevice={activeDevice}
        connectionPhase={devicesState.connectionPhase}
        scanningAdb={devicesState.scanningAdb}
        terminalTabCount={deviceTabs.length}
        runningServiceCount={runningServiceCount}
        onSelectDevice={devicesState.selectDevice}
        onEditDevice={openDeviceModal}
        onRemoveDevice={(id) => void devicesState.removeDevice(id)}
        onOpenAddDevice={() => openDeviceModal(null)}
        onScanAdb={() => void devicesState.scanAdbDevices()}
      />

      <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
        {devicesState.connectionPhase === 'error' && devicesState.connectionError && (
          <div className="border-b border-[var(--border-hairline)] p-3">
            <StatusBanner
              tone="danger"
              title={`无法连接到设备 ${activeDevice?.label ?? ''}`}
              description={devicesState.connectionError}
              action={
                <div className="flex items-center gap-2">
                  {activeDevice && (
                    <Button
                      size="sm"
                      variant="primary"
                      onClick={() => openDeviceModal(activeDevice)}
                    >
                      补充 Token / 编辑设备
                    </Button>
                  )}
                  <Button size="sm" variant="secondary" onClick={() => openDeviceModal(null)}>
                    管理设备连接
                  </Button>
                </div>
              }
            />
          </div>
        )}

        {!activeDevice ? (
          <FirstRunOnboarding
            scanningAdb={devicesState.scanningAdb}
            onOpenAdd={() => openDeviceModal(null)}
            onScanAdb={() => void devicesState.scanAdbDevices()}
          />
        ) : (
          <div className="relative min-h-0 flex-1">
            {activeView === 'containers' && (
              <ContainersView
                device={activeDevice}
                transport={activeTransport}
                daemonStatus={workspace.daemonStatus}
                containers={workspace.containers}
                selectedContainerId={workspace.selectedContainerId}
                onSelectContainer={workspace.setSelectedContainerId}
                servicesByContainer={workspace.servicesByContainer}
                configsByContainer={workspace.configsByContainer}
                loading={workspace.loading}
                onRefresh={workspace.refreshAll}
                onOpenTerminal={handleOpenTerminal}
                onNavigateInstall={() => setActiveView('install')}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'tools' && (
              <ToolsView
                transport={activeTransport}
                containers={workspace.containers}
                selectedContainerId={workspace.selectedContainerId}
                onSelectContainer={workspace.setSelectedContainerId}
                onRefresh={workspace.refreshAll}
                onOpenTerminal={handleOpenTerminal}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'services' && (
              <ServicesView
                transport={activeTransport}
                containers={workspace.containers}
                selectedContainerId={workspace.selectedContainerId}
                onSelectContainer={workspace.setSelectedContainerId}
                servicesByContainer={workspace.servicesByContainer}
                onRefreshServices={workspace.refreshServices}
                onStartContainer={handleStartContainer}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'install' && (
              <InstallView
                transport={activeTransport}
                containers={workspace.containers}
                activeJobs={workspace.activeJobs}
                onDismissJob={workspace.dismissJob}
                onRefresh={workspace.refreshAll}
                onBackToContainers={() => setActiveView('containers')}
                onOpenTerminal={handleOpenTerminal}
                onOpenTools={(id: string) => {
                  workspace.setSelectedContainerId(id);
                  setActiveView('tools');
                }}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'network' && (
              <NetworkSshView
                device={activeDevice}
                transport={activeTransport}
                containers={workspace.containers}
                selectedContainerId={workspace.selectedContainerId}
                onSelectContainer={workspace.setSelectedContainerId}
                webStatus={workspace.webStatus}
                onRefresh={workspace.refreshAll}
                onStartContainer={handleStartContainer}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'logs' && (
              <LogsKernelView
                transport={activeTransport}
                capabilities={workspace.capabilities}
                containers={workspace.containers}
              />
            )}

            {terminalMounted && (
              <Suspense
                fallback={
                  <div className="flex h-full items-center justify-center text-xs text-[var(--text-secondary)]">
                    正在按需加载终端引擎 (xterm.js)…
                  </div>
                }
              >
                <LazyTerminalWorkspace
                  visible={activeView === 'terminal'}
                  device={activeDevice}
                  transport={activeTransport}
                  containers={workspace.containers}
                  tabs={deviceTabs}
                  activeTabId={activeDeviceTabId}
                  selectedContainerId={workspace.selectedContainerId}
                  onSelectTab={setActiveTabId}
                  onNewTab={handleOpenTerminal}
                  onCloseTab={handleCloseTab}
                  onBindSession={handleBindSession}
                />
              </Suspense>
            )}
          </div>
        )}
      </main>

      <AddDeviceModal
        open={deviceModalOpen}
        editingDevice={editingDevice}
        adbDevices={devicesState.adbDevices}
        scanningAdb={devicesState.scanningAdb}
        onClose={() => {
          setDeviceModalOpen(false);
          setEditingDevice(null);
        }}
        onScanAdb={devicesState.scanAdbDevices}
        onConnectAdb={devicesState.connectAdbDevice}
        onSaveManual={devicesState.saveManualDevice}
      />
    </div>
  );
}

/** Root Strata Desktop React application. */
export function App() {
  return (
    <ErrorBoundary>
      <ThemeProvider>
        <ToastProvider>
          <StrataDesktopShell />
        </ToastProvider>
      </ThemeProvider>
    </ErrorBoundary>
  );
}
