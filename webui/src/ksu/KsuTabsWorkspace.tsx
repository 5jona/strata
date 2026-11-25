import { Tabs, TabsContent, TabsList, TabsTrigger } from '@gojona/ui';
import { FileText, Layers, ServerCog, Settings2 } from 'lucide-react';
import { useState } from 'react';
import { ServicesPanel } from '../shared/ServicesPanel.tsx';
import type { ContainerStatusSnapshot } from '../types.ts';
import { KsuConfigTab } from './KsuConfigTab.tsx';
import { KsuDiagnosticsTab } from './KsuDiagnosticsTab.tsx';
import { KsuOverviewTab } from './KsuOverviewTab.tsx';
import type { KsuStrataRuntimeApi } from './useKsuStrataRuntime.ts';

const KSU_TABS = ['overview', 'services', 'config', 'diagnostics'] as const;
type KsuTab = (typeof KSU_TABS)[number];

function isKsuTab(value: string): value is KsuTab {
  return (
    value === 'overview' || value === 'services' || value === 'config' || value === 'diagnostics'
  );
}

export interface KsuTabsWorkspaceProps {
  active: ContainerStatusSnapshot;
  rt: KsuStrataRuntimeApi;
}

/** 4-Tab KernelSU operations workspace: overview/maintenance, services, config/security, diagnostics. */
export function KsuTabsWorkspace({ active, rt }: KsuTabsWorkspaceProps) {
  const [tab, setTab] = useState<KsuTab>('overview');

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
      </TabsList>

      <TabsContent value="overview" className="flex-1 shrink-0">
        <KsuOverviewTab
          container={active}
          busyKey={rt.busyKey}
          onLifecycle={rt.runLifecycle}
          onMaintenance={rt.runMaintenance}
          onCopyText={rt.copyWithFeedback}
        />
      </TabsContent>

      <TabsContent value="services" className="flex-1 shrink-0">
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
      </TabsContent>

      <TabsContent value="config" className="flex-1 shrink-0">
        <KsuConfigTab
          config={rt.containerConfig}
          sshKeys={rt.sshKeys}
          webStatus={rt.webStatus}
          pairingToken={rt.pairingToken}
          busy={rt.busyKey !== null}
          onSaveConfig={rt.patchConfig}
          onAddSshKey={rt.addSshKey}
          onRemoveSshKey={rt.removeSshKey}
          onToggleLan={rt.toggleLan}
          onReadToken={rt.readToken}
          onRotateToken={rt.rotateToken}
          onCopyText={rt.copyWithFeedback}
        />
      </TabsContent>

      <TabsContent value="diagnostics" className="flex-1 shrink-0">
        <KsuDiagnosticsTab
          daemonLogs={rt.daemonLogs}
          dropbearLogs={rt.dropbearLogs}
          capabilities={rt.capabilities}
          busy={rt.busyKey !== null}
          onRefresh={() => rt.refreshDiagnostics(active.id)}
          onCopyLog={rt.copyRawText}
        />
      </TabsContent>
    </Tabs>
  );
}
