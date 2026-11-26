import {
  AppShell,
  Badge,
  BottomNav,
  Button,
  Card,
  EmptyState,
  StatusBanner,
  Tabs,
  TabsContent,
  TabsTrigger,
} from '@gojona/ui';
import { FileText, Layers, Play, RefreshCw, ServerCog, Settings, Settings2 } from 'lucide-react';
import { lazy, Suspense, useEffect, useState } from 'react';
import type { ContainerStatusSnapshot } from '../types.ts';
import { KsuOverviewTab } from './KsuOverviewTab.tsx';
import {
  isKsuTab,
  KSU_TABS,
  type KsuStrataRuntimeApi,
  type KsuTab,
} from './useKsuStrataRuntime.ts';
import { ZeroContainerCard } from './ZeroContainerCard.tsx';

const LazyServicesPanel = lazy(() =>
  import('../shared/ServicesPanel.tsx').then((m) => ({ default: m.ServicesPanel })),
);
const LazyKsuConfigTab = lazy(() =>
  import('./KsuConfigTab.tsx').then((m) => ({ default: m.KsuConfigTab })),
);
const LazyKsuDiagnosticsTab = lazy(() =>
  import('./KsuDiagnosticsTab.tsx').then((m) => ({ default: m.KsuDiagnosticsTab })),
);
const LazyKsuSettingsTab = lazy(() =>
  import('./KsuSettingsTab.tsx').then((m) => ({ default: m.KsuSettingsTab })),
);

export interface KsuTabsWorkspaceProps {
  active: ContainerStatusSnapshot | null;
  daemonOnline: boolean;
  rt: KsuStrataRuntimeApi;
}

const TAB_FALLBACK = (
  <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">正在加载模块…</Card>
);

/** 5-Tab KernelSU operations workspace with bottom navigation dock and 1:1 swipe pager. */
export function KsuTabsWorkspace({ active, daemonOnline, rt }: KsuTabsWorkspaceProps) {
  const [visitedTabs, setVisitedTabs] = useState<ReadonlySet<KsuTab>>(() => new Set(['overview']));

  useEffect(() => {
    setVisitedTabs((prev) => {
      if (prev.has(rt.activeTab)) return prev;
      const next = new Set(prev);
      next.add(rt.activeTab);
      return next;
    });
  }, [rt.activeTab]);

  const noContainerEmpty = (sectionName: string) => (
    <Card className="p-5">
      <EmptyState
        icon={<Layers className="size-7" />}
        title={`暂无容器，无法管理${sectionName}`}
        description="请先前往「概览」自定义容器名称并创建容器，或前往「设置」开启局域网配对连接电脑端 Strata Desktop。"
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button size="sm" variant="primary" onClick={() => rt.setActiveTab('overview')}>
              前往概览创建
            </Button>
            <Button size="sm" variant="secondary" onClick={() => rt.setActiveTab('settings')}>
              前往系统设置
            </Button>
          </div>
        }
      />
    </Card>
  );

  return (
    <Tabs
      className="h-dvh min-h-0"
      value={rt.activeTab}
      onValueChange={(v) => isKsuTab(v) && rt.setActiveTab(v)}
      tabOrder={KSU_TABS}
    >
      <AppShell
        variant="page"
        viewportMode="fill"
        fullBleed
        title="Strata 容器"
        subtitle={
          rt.daemon
            ? `本地容器管理与应急恢复 · 运行 ${rt.daemon.runningContainers}/${rt.daemon.totalContainers}`
            : '本地容器管理与应急恢复'
        }
        bodyClassName="flex flex-col gap-3 px-3 py-3 sm:px-6"
        headerLeading={
          <div className="flex size-9 items-center justify-center rounded-[var(--radius-control)] bg-[var(--accent-soft)] text-[var(--accent)]">
            <Layers className="size-4" />
          </div>
        }
        headerTrailing={
          <div className="flex items-center gap-2">
            <Badge variant={daemonOnline ? 'success' : 'danger'} dot pulse={daemonOnline}>
              {daemonOnline ? '服务在线' : '服务离线'}
            </Badge>
            <Button
              size="sm"
              variant="secondary"
              aria-label="刷新状态"
              disabled={rt.loading}
              onClick={() => void rt.refreshAll()}
            >
              <RefreshCw className={`size-3.5 ${rt.loading ? 'animate-spin' : ''}`} />
              刷新
            </Button>
          </div>
        }
        bottomSlot={
          <BottomNav aria-label="KernelSU 控制台切换">
            <TabsTrigger value="overview" icon={<Layers className="size-4" />}>
              概览
            </TabsTrigger>
            <TabsTrigger value="services" icon={<ServerCog className="size-4" />}>
              服务
            </TabsTrigger>
            <TabsTrigger value="config" icon={<Settings2 className="size-4" />}>
              配置
            </TabsTrigger>
            <TabsTrigger value="diagnostics" icon={<FileText className="size-4" />}>
              诊断
            </TabsTrigger>
            <TabsTrigger value="settings" icon={<Settings className="size-4" />}>
              设置
            </TabsTrigger>
          </BottomNav>
        }
      >
        {!rt.loading && !daemonOnline ? (
          <StatusBanner
            tone="danger"
            title="Strata 后台服务离线或暂不可达"
            description={rt.lastError ?? '无法连接后台服务，请点击右侧按钮尝试启动后台服务。'}
            action={
              <Button
                size="sm"
                variant="primary"
                disabled={rt.busyKey === 'daemon:rescue'}
                onClick={() => void rt.startDaemon()}
              >
                <Play className="size-3.5" />
                启动后台服务
              </Button>
            }
          />
        ) : null}

        {rt.containers.length > 1 ? (
          <div
            data-swipe-ignore="true"
            className="flex shrink-0 items-center gap-1.5 overflow-x-auto pb-0.5 [scrollbar-width:none]"
          >
            {rt.containers.map((c) => (
              <Button
                key={c.id}
                className="shrink-0"
                size="sm"
                variant={c.id === rt.selectedId ? 'pill' : 'secondary'}
                onClick={() => rt.selectContainer(c.id)}
              >
                {c.id}
                {c.isDefault ? ' (默认)' : ''}
              </Button>
            ))}
          </div>
        ) : null}

        {rt.loading && !active ? (
          <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
            正在读取 Strata 容器状态…
          </Card>
        ) : (
          <>
            <TabsContent value="overview" className="flex-1 shrink-0">
              {active ? (
                <KsuOverviewTab
                  container={active}
                  containers={rt.containers}
                  selectedId={rt.selectedId}
                  busyKey={rt.busyKey}
                  onSelectContainer={rt.selectContainer}
                  onLifecycle={rt.runLifecycle}
                  onMaintenance={rt.runMaintenance}
                  onSetDefault={rt.setDefaultContainer}
                  onDeleteContainer={rt.deleteContainer}
                  onCreateContainer={rt.createContainer}
                  onCopyText={rt.copyWithFeedback}
                  onOpenSettings={() => rt.setActiveTab('settings')}
                />
              ) : (
                <ZeroContainerCard
                  daemonOnline={daemonOnline}
                  busy={rt.busyKey !== null}
                  onOpenSettings={() => rt.setActiveTab('settings')}
                  onCopyText={rt.copyWithFeedback}
                  onCreateContainer={rt.createContainer}
                />
              )}
            </TabsContent>

            <TabsContent value="services" className="flex-1 shrink-0">
              {visitedTabs.has('services') ? (
                <Suspense fallback={TAB_FALLBACK}>
                  {active ? (
                    <LazyServicesPanel
                      containerId={active.id}
                      containerRunning={active.phase === 'running'}
                      services={rt.services}
                      serviceLogs={rt.serviceLogs}
                      busyKey={rt.busyKey}
                      onRefresh={() => rt.refreshContainerDetails(active.id)}
                      onAction={rt.runServiceAction}
                      onToggleAutoStart={rt.toggleServiceAutoStart}
                      onLoadLogs={rt.loadServiceLogs}
                      onCopyText={rt.copyWithFeedback}
                    />
                  ) : (
                    noContainerEmpty('托管服务')
                  )}
                </Suspense>
              ) : (
                TAB_FALLBACK
              )}
            </TabsContent>

            <TabsContent value="config" className="flex-1 shrink-0">
              {visitedTabs.has('config') ? (
                <Suspense fallback={TAB_FALLBACK}>
                  {active ? (
                    <LazyKsuConfigTab
                      config={rt.containerConfig}
                      sshKeys={rt.sshKeys}
                      busy={rt.busyKey !== null}
                      onSaveConfig={rt.patchConfig}
                      onAddSshKey={rt.addSshKey}
                      onRemoveSshKey={rt.removeSshKey}
                    />
                  ) : (
                    noContainerEmpty('容器配置')
                  )}
                </Suspense>
              ) : (
                TAB_FALLBACK
              )}
            </TabsContent>

            <TabsContent value="diagnostics" className="flex-1 shrink-0">
              {visitedTabs.has('diagnostics') ? (
                <Suspense fallback={TAB_FALLBACK}>
                  <LazyKsuDiagnosticsTab
                    daemonLogs={rt.daemonLogs}
                    dropbearLogs={rt.dropbearLogs}
                    capabilities={rt.capabilities}
                    hasContainer={active !== null}
                    busy={rt.busyKey !== null}
                    onRefresh={() => rt.refreshDiagnostics(active?.id)}
                    onCopyLog={rt.copyRawText}
                  />
                </Suspense>
              ) : (
                TAB_FALLBACK
              )}
            </TabsContent>

            <TabsContent value="settings" className="flex-1 shrink-0">
              {visitedTabs.has('settings') ? (
                <Suspense fallback={TAB_FALLBACK}>
                  <LazyKsuSettingsTab
                    webStatus={rt.webStatus}
                    pairingToken={rt.pairingToken}
                    busy={rt.busyKey !== null}
                    onToggleLan={rt.toggleLan}
                    onReadToken={rt.readToken}
                    onRotateToken={rt.rotateToken}
                    onCopyText={rt.copyWithFeedback}
                  />
                </Suspense>
              ) : (
                TAB_FALLBACK
              )}
            </TabsContent>
          </>
        )}
      </AppShell>
    </Tabs>
  );
}
