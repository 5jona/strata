import type { BridgeOptions } from '@pochita/bridge';
import {
  SideVeilPluginProvider,
  useContainerAction,
  useHaptics,
  usePluginBackendState,
  usePresentationMode,
} from '@pochita/sideveil-sdk';
import {
  AppShell,
  Button,
  Card,
  cn,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  ThemeProvider,
  ToastProvider,
  useToast,
} from '@pochita/ui';
import { Layers, Settings2, Terminal } from 'lucide-react';
import { useCallback, useMemo, useState } from 'react';
import { ConfigTab } from './components/ConfigTab.tsx';
import { OverviewTab } from './components/OverviewTab.tsx';
import { StrataHeaderActions } from './components/StrataHeaderActions.tsx';
import { TasksTab } from './components/TasksTab.tsx';
import { useStrataPluginTheme } from './hooks/useStrataPluginTheme.ts';
import { useStrataRuntime } from './hooks/useStrataRuntime.ts';
import { createMockStrataTransport } from './lib/mockStrataBridge.ts';
import { STRATA_MANIFEST, STRATA_PLUGIN_ID } from './manifest.ts';

const STRATA_TAB_ORDER = ['overview', 'tasks', 'config'] as const;
type StrataTab = (typeof STRATA_TAB_ORDER)[number];

const PINNED_DEFAULT_BOUNDS = { widthDp: 344, heightDp: 520 };

function isStrataTab(value: string): value is StrataTab {
  return value === 'overview' || value === 'tasks' || value === 'config';
}

function StrataWorkbench() {
  const { isNativeHost, isDark, copyText } = useStrataPluginTheme();
  const { dismiss, requestImeFocus } = useContainerAction();
  const backendState = usePluginBackendState();
  const { mode, isPinned, clickThrough, setMode, setClickThrough } = usePresentationMode('drawer');
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const [tab, setTab] = useState<StrataTab>('overview');
  const backendOnline = !isNativeHost || backendState === 'online';
  const runtime = useStrataRuntime(backendOnline, isDark);

  const handleImeFocusChange = useCallback(
    (focused: boolean) => {
      void requestImeFocus(focused).catch(() => {});
    },
    [requestImeFocus],
  );

  const handleCycleMode = useCallback(() => {
    void triggerHaptic('click');
    const next =
      mode === 'drawer'
        ? setMode('wide')
        : mode === 'wide'
          ? setMode('pinned', PINNED_DEFAULT_BOUNDS)
          : setMode('drawer');
    void next.catch(() => {});
  }, [mode, setMode, triggerHaptic]);

  const handleToggleClickThrough = useCallback(() => {
    void triggerHaptic('tick');
    void setClickThrough(!clickThrough).catch(() => {});
  }, [clickThrough, setClickThrough, triggerHaptic]);

  const handleCopySsh = useCallback(
    async (sshCommand: string) => {
      const ok = await copyText(sshCommand);
      pushToast(ok ? `已复制：${sshCommand}` : '复制失败，请手动复制', ok ? 'success' : 'danger');
    },
    [copyText, pushToast],
  );

  const handleSelectContainer = useCallback(
    (containerId: string) => {
      runtime.setSelectedId(containerId);
      void runtime.refreshConfigAndLogs(containerId);
    },
    [runtime],
  );

  return (
    <div
      className={cn(
        'flex w-full flex-col',
        isNativeHost
          ? 'h-dvh overflow-hidden'
          : mode === 'wide'
            ? 'mx-auto min-h-dvh max-w-[680px] p-2'
            : 'mx-auto min-h-dvh max-w-[380px] p-2',
      )}
    >
      <AppShell
        variant="drawer"
        title="Strata 容器"
        className={
          isNativeHost
            ? 'h-full min-h-0 flex-1'
            : 'min-h-[calc(100dvh-1rem)] flex-1 rounded-[var(--radius-panel)]'
        }
        headerLeading={
          <div className="strata-header-icon flex size-7 items-center justify-center rounded-[var(--radius-control)] bg-[var(--accent-soft)] text-[var(--accent)]">
            <Layers className="size-3.5" />
          </div>
        }
        headerTrailing={
          <StrataHeaderActions
            isNativeHost={isNativeHost}
            backendOnline={backendOnline}
            backendState={backendState}
            mode={mode}
            isPinned={isPinned}
            clickThrough={clickThrough}
            loading={runtime.loading}
            onToggleClickThrough={handleToggleClickThrough}
            onCycleMode={handleCycleMode}
            onRefresh={() => void runtime.refreshAll()}
            onDismiss={() => void dismiss()}
          />
        }
      >
        <div className="flex flex-col gap-2.5">
          {runtime.containers.length > 1 ? (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
              {runtime.containers.map((item) => (
                <Button
                  key={item.id}
                  className="shrink-0"
                  size="sm"
                  variant={item.id === runtime.selectedId ? 'pill' : 'secondary'}
                  aria-label={`选择容器 ${item.id}`}
                  onClick={() => handleSelectContainer(item.id)}
                >
                  {item.id}
                  {item.isDefault ? ' (主力)' : ''}
                </Button>
              ))}
            </div>
          ) : null}

          {!backendOnline ? (
            <Card className="flex flex-col gap-1.5 p-3">
              <span className="text-xs font-medium text-[var(--warning)]">
                等待 stratad 守护进程接入
              </span>
              <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
                Strata 守护进程支持完全独立运行；当 `stratad` 启动并开启 SideVeil 桥接后会自动上线。
              </p>
            </Card>
          ) : runtime.lastError ? (
            <Card className="flex flex-col gap-2 p-3">
              <span className="text-xs font-medium text-[var(--danger)]">状态读取失败</span>
              <p className="font-mono text-[11px] text-[var(--text-secondary)]">
                {runtime.lastError}
              </p>
              <Button
                size="sm"
                variant="secondary"
                aria-label="重试加载状态"
                onClick={() => void runtime.refreshAll()}
              >
                重试连接
              </Button>
            </Card>
          ) : runtime.activeContainer ? (
            <Tabs
              value={tab}
              onValueChange={(v) => isStrataTab(v) && setTab(v)}
              tabOrder={STRATA_TAB_ORDER}
            >
              <TabsList aria-label="Strata 管理视图切换">
                <TabsTrigger value="overview">
                  <Layers className="size-3.5" />
                  总览
                </TabsTrigger>
                <TabsTrigger value="tasks">
                  <Terminal className="size-3.5" />
                  任务
                </TabsTrigger>
                <TabsTrigger value="config">
                  <Settings2 className="size-3.5" />
                  日志配置
                </TabsTrigger>
              </TabsList>

              <TabsContent value="overview">
                <OverviewTab
                  container={runtime.activeContainer}
                  hudState={runtime.hudState}
                  busyAction={runtime.busyAction}
                  onLifecycle={runtime.runLifecycle}
                  onMaintenance={runtime.runMaintenance}
                  onUpdateHud={runtime.updateHud}
                  onCopySsh={handleCopySsh}
                />
              </TabsContent>

              <TabsContent value="tasks">
                <TasksTab
                  containerRunning={runtime.activeContainer.phase === 'running'}
                  tasks={runtime.tasks}
                  activeTaskId={runtime.activeTaskId}
                  entries={runtime.taskEntries}
                  busy={runtime.busyAction !== null}
                  onRunTask={runtime.runTask}
                  onCancelTask={runtime.cancelTask}
                  onSelectTask={runtime.selectTask}
                  onClearConsole={runtime.clearTaskConsole}
                  onCopyLog={copyText}
                  onImeFocusChange={handleImeFocusChange}
                />
              </TabsContent>

              <TabsContent value="config">
                <ConfigTab
                  config={runtime.containerConfig}
                  dropbearLogs={runtime.dropbearLogs}
                  sshKeys={runtime.sshKeys}
                  busy={runtime.busyAction !== null}
                  onRefresh={() => runtime.refreshConfigAndLogs()}
                  onSaveConfig={runtime.patchConfig}
                  onAddSshKey={runtime.addSshKey}
                  onCopyLog={copyText}
                  onImeFocusChange={handleImeFocusChange}
                />
              </TabsContent>
            </Tabs>
          ) : (
            <Card className="p-4 text-center text-xs text-[var(--text-secondary)]">
              正在读取 Strata 容器状态…
            </Card>
          )}
        </div>
      </AppShell>
    </div>
  );
}

/** Root SideVeil plugin component wiring bridge, theme, and toast providers for Strata Manager. */
export function StrataManagerApp() {
  const bridgeOptions = useMemo<BridgeOptions>(
    () => ({
      allowUrlOverride: false,
      allowedOrigins: [`https://${STRATA_PLUGIN_ID}.plugins.sideveil.local`],
      mockNamespace: 'strata-manager',
      mockTransport: createMockStrataTransport(),
    }),
    [],
  );

  return (
    <SideVeilPluginProvider
      pluginId={STRATA_PLUGIN_ID}
      manifest={STRATA_MANIFEST}
      bridgeOptions={bridgeOptions}
    >
      <ThemeProvider
        defaultTheme="coloros-glass"
        defaultColorMode="system"
        defaultSurfaceMode="native-glass"
        syncEnabled={false}
      >
        <ToastProvider>
          <StrataWorkbench />
        </ToastProvider>
      </ThemeProvider>
    </SideVeilPluginProvider>
  );
}
