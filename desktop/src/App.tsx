import {
  Button,
  EmptyState,
  ErrorBoundary,
  StatusBanner,
  ThemeProvider,
  ToastProvider,
} from '@gojona/ui';
import { Smartphone } from 'lucide-react';
import { lazy, Suspense, useCallback, useMemo, useState } from 'react';
import { AddDeviceModal } from './components/AddDeviceModal.tsx';
import { ContainersView } from './components/ContainersView.tsx';
import { InstallView } from './components/InstallView.tsx';
import { LogsKernelView } from './components/LogsKernelView.tsx';
import { NetworkSshView } from './components/NetworkSshView.tsx';
import { ServicesView } from './components/ServicesView.tsx';
import { Sidebar } from './components/Sidebar.tsx';
import { useDesktopDevices } from './hooks/useDesktopDevices.ts';
import { useStrataWorkspace } from './hooks/useStrataWorkspace.ts';
import type { NavViewId, TerminalTabSpec } from './types/device.ts';

const LazyTerminalWorkspace = lazy(() => import('./components/TerminalWorkspace.tsx'));

function StrataDesktopShell() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [activeView, setActiveView] = useState<NavViewId>('containers');
  const [addDeviceOpen, setAddDeviceOpen] = useState(false);
  const [terminalMounted, setTerminalMounted] = useState(false);
  const [tabs, setTabs] = useState<TerminalTabSpec[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  const devicesState = useDesktopDevices();
  const workspace = useStrataWorkspace(devicesState.transport);

  const deviceId = devicesState.activeDevice?.id ?? 'none';
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
      setTabs((prev) => [...prev, { id, deviceId, containerId, title }]);
      setActiveTabId(id);
      setTerminalMounted(true);
      setActiveView('terminal');
    },
    [deviceId, tabs],
  );

  const handleCloseTab = useCallback(
    (tabId: string) => {
      const target = tabs.find((t) => t.id === tabId);
      if (target?.sessionId && devicesState.transport) {
        void devicesState.transport.killPty(target.sessionId).catch(() => {});
      }
      setTabs((prev) => prev.filter((t) => t.id !== tabId));
      if (activeTabId === tabId) {
        setActiveTabId(null);
      }
    },
    [activeTabId, devicesState.transport, tabs],
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

  return (
    <div className="flex h-full w-full overflow-hidden bg-[var(--surface-canvas)] text-[var(--text-primary)]">
      <Sidebar
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        activeView={activeView}
        onSelectView={handleSelectView}
        devices={devicesState.devices}
        activeDevice={devicesState.activeDevice}
        connectionPhase={devicesState.connectionPhase}
        scanningAdb={devicesState.scanningAdb}
        terminalTabCount={deviceTabs.length}
        runningServiceCount={runningServiceCount}
        onSelectDevice={devicesState.selectDevice}
        onRemoveDevice={(id) => void devicesState.removeDevice(id)}
        onOpenAddDevice={() => setAddDeviceOpen(true)}
        onScanAdb={() => void devicesState.scanAdbDevices()}
      />

      <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
        {devicesState.connectionPhase === 'error' && devicesState.connectionError && (
          <div className="border-b border-[var(--border-hairline)] p-3">
            <StatusBanner
              tone="danger"
              title={`无法连接到设备 ${devicesState.activeDevice?.label ?? ''}`}
              description={devicesState.connectionError}
              action={
                <Button size="sm" variant="secondary" onClick={() => setAddDeviceOpen(true)}>
                  管理设备连接
                </Button>
              }
            />
          </div>
        )}

        {!devicesState.activeDevice ? (
          <div className="flex flex-1 items-center justify-center p-8">
            <EmptyState
              icon={<Smartphone className="size-10" />}
              title="尚未选择 Android 设备"
              description="通过 USB / 无线 ADB 自动发现并建立隧道，或手动添加局域网设备。"
              action={
                <Button variant="primary" onClick={() => setAddDeviceOpen(true)}>
                  扫描 ADB / 添加设备
                </Button>
              }
            />
          </div>
        ) : (
          <div className="relative min-h-0 flex-1">
            {activeView === 'containers' && (
              <ContainersView
                device={devicesState.activeDevice}
                transport={devicesState.transport}
                daemonStatus={workspace.daemonStatus}
                containers={workspace.containers}
                servicesByContainer={workspace.servicesByContainer}
                loading={workspace.loading}
                onRefresh={workspace.refreshAll}
                onOpenTerminal={handleOpenTerminal}
                onNavigateInstall={() => setActiveView('install')}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'services' && (
              <ServicesView
                transport={devicesState.transport}
                containers={workspace.containers}
                servicesByContainer={workspace.servicesByContainer}
                onRefreshServices={workspace.refreshServices}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'install' && (
              <InstallView
                transport={devicesState.transport}
                activeJobs={workspace.activeJobs}
                onRefresh={workspace.refreshAll}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'network' && (
              <NetworkSshView
                device={devicesState.activeDevice}
                transport={devicesState.transport}
                containers={workspace.containers}
                webStatus={workspace.webStatus}
                onRefresh={workspace.refreshAll}
                runAction={workspace.runAction}
              />
            )}

            {activeView === 'logs' && (
              <LogsKernelView
                transport={devicesState.transport}
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
                  device={devicesState.activeDevice}
                  transport={devicesState.transport}
                  containers={workspace.containers}
                  tabs={deviceTabs}
                  activeTabId={activeDeviceTabId}
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
        open={addDeviceOpen}
        adbDevices={devicesState.adbDevices}
        scanningAdb={devicesState.scanningAdb}
        onClose={() => setAddDeviceOpen(false)}
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
