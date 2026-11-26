import { Button, Card, EmptyState, Tabs, TabsContent, TabsList, TabsTrigger } from '@gojona/ui';
import { FileText, Layers, ServerCog, Settings, Settings2 } from 'lucide-react';
import { useState } from 'react';
import { ServicesPanel } from '../shared/ServicesPanel.tsx';
import type { ContainerStatusSnapshot } from '../types.ts';
import { KsuConfigTab } from './KsuConfigTab.tsx';
import { KsuDiagnosticsTab } from './KsuDiagnosticsTab.tsx';
import { KsuOverviewTab } from './KsuOverviewTab.tsx';
import { KsuSettingsTab } from './KsuSettingsTab.tsx';
import type { KsuStrataRuntimeApi } from './useKsuStrataRuntime.ts';
import { ZeroContainerCard } from './ZeroContainerCard.tsx';

const KSU_TABS = ['overview', 'services', 'config', 'diagnostics', 'settings'] as const;
type KsuTab = (typeof KSU_TABS)[number];

function isKsuTab(value: string): value is KsuTab {
  return (
    value === 'overview' ||
    value === 'services' ||
    value === 'config' ||
    value === 'diagnostics' ||
    value === 'settings'
  );
}

export interface KsuTabsWorkspaceProps {
  active: ContainerStatusSnapshot | null;
  daemonOnline: boolean;
  rt: KsuStrataRuntimeApi;
}

/** 5-Tab KernelSU operations workspace: overview, services, config/security, diagnostics, global settings. */
export function KsuTabsWorkspace({ active, daemonOnline, rt }: KsuTabsWorkspaceProps) {
  const [tab, setTab] = useState<KsuTab>('overview');

  const noContainerEmpty = (sectionName: string) => (
    <Card className="p-5">
      <EmptyState
        icon={<Layers className="size-7" />}
        title={`暂无容器，无法管理${sectionName}`}
        description="请先前往「系统设置」开启局域网配对并连接电脑端 Strata Desktop 创建容器，或在「概览与维护」查看初始化命令。"
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button size="sm" variant="primary" onClick={() => setTab('settings')}>
              前往系统设置
            </Button>
            <Button size="sm" variant="secondary" onClick={() => setTab('overview')}>
              查看创建引导
            </Button>
          </div>
        }
      />
    </Card>
  );

  return (
    <Tabs
      className="flex flex-1 shrink-0 flex-col"
      value={tab}
      onValueChange={(v) => isKsuTab(v) && setTab(v)}
      tabOrder={KSU_TABS}
    >
      <TabsList className="shrink-0" aria-label="KernelSU 控制台切换">
        <TabsTrigger value="overview">
          <Layers className="size-3.5" />
          概览与维护
        </TabsTrigger>
        <TabsTrigger value="services">
          <ServerCog className="size-3.5" />
          服务
        </TabsTrigger>
        <TabsTrigger value="config">
          <Settings2 className="size-3.5" />
          配置与安全
        </TabsTrigger>
        <TabsTrigger value="diagnostics">
          <FileText className="size-3.5" />
          日志与环境
        </TabsTrigger>
        <TabsTrigger value="settings">
          <Settings className="size-3.5" />
          系统设置
        </TabsTrigger>
      </TabsList>

      <TabsContent value="overview" className="flex-1 shrink-0">
        {active ? (
          <KsuOverviewTab
            container={active}
            busyKey={rt.busyKey}
            onLifecycle={rt.runLifecycle}
            onMaintenance={rt.runMaintenance}
            onCopyText={rt.copyWithFeedback}
            onOpenSettings={() => setTab('settings')}
          />
        ) : (
          <ZeroContainerCard
            daemonOnline={daemonOnline}
            onOpenSettings={() => setTab('settings')}
            onCopyText={rt.copyWithFeedback}
          />
        )}
      </TabsContent>

      <TabsContent value="services" className="flex-1 shrink-0">
        {active ? (
          <ServicesPanel
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
      </TabsContent>

      <TabsContent value="config" className="flex-1 shrink-0">
        {active ? (
          <KsuConfigTab
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
      </TabsContent>

      <TabsContent value="diagnostics" className="flex-1 shrink-0">
        <KsuDiagnosticsTab
          daemonLogs={rt.daemonLogs}
          dropbearLogs={rt.dropbearLogs}
          capabilities={rt.capabilities}
          hasContainer={active !== null}
          busy={rt.busyKey !== null}
          onRefresh={() => rt.refreshDiagnostics(active?.id)}
          onCopyLog={rt.copyRawText}
        />
      </TabsContent>

      <TabsContent value="settings" className="flex-1 shrink-0">
        <KsuSettingsTab
          webStatus={rt.webStatus}
          pairingToken={rt.pairingToken}
          busy={rt.busyKey !== null}
          onToggleLan={rt.toggleLan}
          onReadToken={rt.readToken}
          onRotateToken={rt.rotateToken}
          onCopyText={rt.copyWithFeedback}
        />
      </TabsContent>
    </Tabs>
  );
}
