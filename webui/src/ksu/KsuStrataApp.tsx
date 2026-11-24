import { type BridgeOptions, BridgeProvider } from '@gojona/bridge';
import {
  AppShell,
  Badge,
  Button,
  Card,
  ErrorBoundary,
  MetricTile,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  ThemeProvider,
  ThemeSwitcher,
  ToastProvider,
} from '@gojona/ui';
import {
  Copy,
  HardDrive,
  Laptop,
  Layers,
  Play,
  RefreshCw,
  RotateCw,
  ServerCog,
  Square,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { PcPairingCard } from '../components/PcPairingCard.tsx';
import { ServicesPanel } from '../components/ServicesPanel.tsx';
import {
  buildSshCommand,
  formatBytes,
  formatUptime,
  PHASE_META,
  splitBytes,
} from '../lib/strataFormat.ts';
import { createMockKsuStrataTransport } from './ksuStrataClient.ts';
import { useKsuStrataRuntime } from './useKsuStrataRuntime.ts';

const KSU_TABS = ['containers', 'services', 'pairing'] as const;
type KsuTab = (typeof KSU_TABS)[number];

function isKsuTab(value: string): value is KsuTab {
  return value === 'containers' || value === 'services' || value === 'pairing';
}

function KsuStrataWorkbench() {
  const [tab, setTab] = useState<KsuTab>('containers');
  const rt = useKsuStrataRuntime();
  const active = rt.activeContainer;

  return (
    <AppShell
      variant="page"
      maxWidthClassName="max-w-none"
      title="Strata 容器"
      subtitle={
        rt.daemon
          ? `stratad PID ${rt.daemon.pid} · 运行 ${rt.daemon.runningContainers}/${rt.daemon.totalContainers}`
          : 'KernelSU 独立控制台'
      }
      bodyClassName="flex flex-col px-3 py-3 sm:px-5"
      headerLeading={
        <div className="flex size-9 items-center justify-center rounded-[var(--radius-control)] bg-[var(--accent-soft)] text-[var(--accent)]">
          <Layers className="size-4" />
        </div>
      }
      headerTrailing={
        <div className="flex items-center gap-2">
          {rt.daemon ? (
            <Badge variant="success" dot pulse>
              stratad 在线
            </Badge>
          ) : null}
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
    >
      <div className="flex w-full grow shrink-0 flex-col gap-3">
        {rt.containers.length > 1 ? (
          <div className="flex shrink-0 flex-wrap gap-1.5">
            {rt.containers.map((c) => (
              <Button
                key={c.id}
                size="sm"
                variant={c.id === rt.selectedId ? 'pill' : 'secondary'}
                onClick={() => {
                  rt.setSelectedId(c.id);
                  void rt.refreshServices(c.id);
                }}
              >
                {c.id}
              </Button>
            ))}
          </div>
        ) : null}

        {rt.lastError ? (
          <Card className="flex flex-col gap-2 p-4">
            <span className="text-xs font-medium text-[var(--danger)]">stratad 连接失败</span>
            <p className="font-mono text-[11px] text-[var(--text-secondary)]">{rt.lastError}</p>
            <Button size="sm" variant="secondary" onClick={() => void rt.refreshAll()}>
              重试连接
            </Button>
          </Card>
        ) : active ? (
          <Tabs
            className="flex flex-1 shrink-0 flex-col"
            value={tab}
            onValueChange={(v) => isKsuTab(v) && setTab(v)}
            tabOrder={KSU_TABS}
          >
            <TabsList className="shrink-0" aria-label="KernelSU 控制台切换">
              <TabsTrigger value="containers">
                <Layers className="size-3.5" />
                容器
              </TabsTrigger>
              <TabsTrigger value="services">
                <ServerCog className="size-3.5" />
                服务
              </TabsTrigger>
              <TabsTrigger value="pairing">
                <Laptop className="size-3.5" />
                PC 配对
              </TabsTrigger>
            </TabsList>

            <TabsContent value="containers" className="flex-1 shrink-0">
              <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
                <div className="flex flex-col gap-3 md:col-span-5">
                  <Card className="flex flex-col gap-3 p-4">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-base font-semibold text-[var(--text-primary)]">
                          {active.hostname}
                        </span>
                        <Badge
                          variant={PHASE_META[active.phase].variant}
                          dot
                          pulse={active.phase === 'running'}
                        >
                          {PHASE_META[active.phase].label}
                        </Badge>
                      </div>
                      <p className="mt-1 font-mono text-xs text-[var(--text-secondary)]">
                        {active.phase === 'running'
                          ? `PID ${active.nsPid ?? '-'} · 运行 ${formatUptime(active.uptimeMs)}`
                          : active.imagePath}
                      </p>
                    </div>

                    <div className="grid grid-cols-3 gap-2">
                      <Button
                        size="sm"
                        variant={active.phase === 'running' ? 'secondary' : 'primary'}
                        disabled={rt.busyKey !== null || active.phase === 'running'}
                        onClick={() => void rt.runLifecycle('start')}
                      >
                        <Play className="size-3.5" />
                        启动
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={rt.busyKey !== null || active.phase === 'stopped'}
                        onClick={() => void rt.runLifecycle('stop')}
                      >
                        <Square className="size-3.5" />
                        停止
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={rt.busyKey !== null}
                        onClick={() => void rt.runLifecycle('restart')}
                      >
                        <RotateCw className="size-3.5" />
                        重启
                      </Button>
                    </div>
                  </Card>

                  <Card className="flex items-center justify-between gap-2 p-4">
                    <div className="min-w-0">
                      <p className="text-[11px] text-[var(--text-secondary)]">SSH 快捷连接</p>
                      <p className="mt-0.5 truncate font-mono text-xs text-[var(--text-primary)]">
                        {buildSshCommand(active)}
                      </p>
                    </div>
                    <Button
                      className="shrink-0"
                      size="sm"
                      variant="pill"
                      onClick={() => void rt.copyWithFeedback(buildSshCommand(active), 'SSH 命令')}
                    >
                      <Copy className="size-3.5" />
                      复制
                    </Button>
                  </Card>
                </div>

                <div className="grid grid-cols-2 gap-3 md:col-span-7">
                  <MetricTile
                    label="镜像占用"
                    value={splitBytes(active.disk.imageAllocatedBytes).value}
                    unit={splitBytes(active.disk.imageAllocatedBytes).unit}
                    hint={`上限 ${formatBytes(active.disk.imageVirtualBytes)}`}
                    icon={<HardDrive className="size-3.5" />}
                  />
                  <MetricTile
                    label="Dropbear SSH"
                    value={`:${active.sshPort}`}
                    hint={active.primaryIpv4}
                    statusLabel={active.sshEnabled ? '启用' : '关闭'}
                    statusVariant={active.sshEnabled ? 'success' : 'neutral'}
                  />
                  <MetricTile
                    label="根文件系统"
                    value={splitBytes(active.disk.rootfsUsedBytes ?? 0).value}
                    unit={splitBytes(active.disk.rootfsUsedBytes ?? 0).unit}
                    hint={`可用 ${formatBytes(active.disk.rootfsAvailableBytes ?? 0)}`}
                  />
                  <MetricTile
                    label="运行模式"
                    value={active.hostBridge ? 'Bridge' : 'Isolated'}
                    hint={active.mountDir}
                    statusLabel={active.mounted ? '已挂载' : '未挂载'}
                    statusVariant={active.mounted ? 'success' : 'neutral'}
                  />
                </div>
              </div>
            </TabsContent>

            <TabsContent value="services" className="flex-1 shrink-0">
              <ServicesPanel
                containerId={active.id}
                containerRunning={active.phase === 'running'}
                services={rt.services}
                serviceLogs={rt.serviceLogs}
                busyKey={rt.busyKey}
                onRefresh={() => rt.refreshServices(active.id)}
                onAction={rt.runServiceAction}
                onLoadLogs={rt.loadServiceLogs}
                onCopyText={rt.copyWithFeedback}
              />
            </TabsContent>

            <TabsContent value="pairing" className="flex-1 shrink-0">
              <div className="grid grid-cols-1 gap-3 md:grid-cols-12">
                <div className="md:col-span-8">
                  <PcPairingCard
                    webStatus={rt.webStatus}
                    pairingToken={rt.pairingToken}
                    busy={rt.busyKey !== null}
                    onToggleLan={rt.toggleLan}
                    onReadToken={rt.readToken}
                    onRotateToken={rt.rotateToken}
                    onCopyText={rt.copyWithFeedback}
                  />
                </div>
                <Card className="flex flex-col gap-2 p-4 md:col-span-4">
                  <span className="text-xs font-medium text-[var(--text-primary)]">界面主题</span>
                  <ThemeSwitcher compact />
                </Card>
              </div>
            </TabsContent>
          </Tabs>
        ) : (
          <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
            正在读取 Strata 容器状态…
          </Card>
        )}
      </div>
    </AppShell>
  );
}

/** Root standalone KernelSU WebUI component (`strata/webroot/`). */
export function KsuStrataApp() {
  const bridgeOptions = useMemo<BridgeOptions>(
    () => ({
      mockNamespace: 'strata-ksu-webui',
      mockTransport: createMockKsuStrataTransport(),
    }),
    [],
  );

  return (
    <ErrorBoundary>
      <BridgeProvider options={bridgeOptions}>
        <ThemeProvider>
          <ToastProvider>
            <KsuStrataWorkbench />
          </ToastProvider>
        </ThemeProvider>
      </BridgeProvider>
    </ErrorBoundary>
  );
}
