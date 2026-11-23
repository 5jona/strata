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
  Badge,
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
import {
  Layers,
  Lock,
  Maximize2,
  PanelRight,
  Pin,
  RefreshCw,
  Settings2,
  Terminal,
  Unlock,
  X,
} from 'lucide-react';
import { useCallback, useMemo, useState } from 'react';
import { ConfigTab } from './components/ConfigTab.tsx';
import { OverviewTab } from './components/OverviewTab.tsx';
import { TasksTab } from './components/TasksTab.tsx';
import { useStrataPluginTheme } from './hooks/useStrataPluginTheme.ts';
import { useStrataRuntime } from './hooks/useStrataRuntime.ts';
import { createMockStrataTransport } from './lib/mockStrataBridge.ts';
import { STRATA_MANIFEST, STRATA_PLUGIN_ID } from './manifest.ts';

const PINNED_DEFAULT_BOUNDS = { widthDp: 344, heightDp: 520 };

function StrataWorkbench() {
  const { isNativeHost, isDark, copyText } = useStrataPluginTheme();
  const { dismiss, requestImeFocus } = useContainerAction();
  const backendState = usePluginBackendState();
  const { mode, isPinned, clickThrough, setMode, setClickThrough } = usePresentationMode('drawer');
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const [tab, setTab] = useState<'overview' | 'tasks' | 'config'>('overview');
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
    if (mode === 'drawer') {
      void setMode('wide').catch(() => {});
    } else if (mode === 'wide') {
      void setMode('pinned', PINNED_DEFAULT_BOUNDS).catch(() => {});
    } else {
      void setMode('drawer').catch(() => {});
    }
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

  const modeLabel = mode === 'wide' ? '切换到悬浮窗' : isPinned ? '返回侧边抽屉' : '切换到宽屏模式';
  const ModeIcon = mode === 'wide' ? Pin : isPinned ? PanelRight : Maximize2;

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
          <div className="strata-header-actions flex items-center gap-1">
            <Badge variant={backendOnline ? 'success' : 'warning'} dot pulse={backendOnline}>
              {!isNativeHost ? '预览' : backendState === 'online' ? '在线' : '离线'}
            </Badge>
            {isPinned ? (
              <Button
                size="icon-sm"
                variant={clickThrough ? 'pill' : 'ghost'}
                aria-label={clickThrough ? '退出点击穿透' : '开启浮窗点击穿透'}
                onClick={handleToggleClickThrough}
              >
                {clickThrough ? <Lock className="size-3.5" /> : <Unlock className="size-3.5" />}
              </Button>
            ) : null}
            <Button size="icon-sm" variant="ghost" aria-label={modeLabel} onClick={handleCycleMode}>
              <ModeIcon className="size-3.5" />
            </Button>
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label="刷新容器状态"
              disabled={runtime.loading}
              onClick={() => void runtime.refreshAll()}
            >
              <RefreshCw className={cn('size-3.5', runtime.loading && 'animate-spin')} />
            </Button>
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label="关闭 Strata 面板"
              onClick={() => void dismiss()}
            >
              <X className="size-4" />
            </Button>
          </div>
        }
      >
        <div className="flex flex-col gap-2.5">
          {runtime.containers.length > 1 ? (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
              {runtime.containers.map((item) => (
                <Button
                  key={item.id}
                  size="sm"
                  variant={item.id === runtime.selectedId ? 'pill' : 'secondary'}
                  aria-label={`选择容器 ${item.id}`}
                  onClick={() => {
                    runtime.setSelectedId(item.id);
                    void runtime.refreshConfigAndLogs(item.id);
                  }}
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
            <Tabs value={tab} onValueChange={(v) => setTab(v as 'overview' | 'tasks' | 'config')}>
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

export function StrataManagerApp() {
  const bridgeOptions = useMemo<BridgeOptions>(() => {
    const mockTransport = createMockStrataTransport();
    return {
      allowUrlOverride: false,
      allowedOrigins: [`https://${STRATA_PLUGIN_ID}.plugins.sideveil.local`],
      mockNamespace: 'strata-manager',
      mockTransport,
    };
  }, []);

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
