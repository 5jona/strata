import type { BridgeOptions } from '@gojona/bridge';
import {
  SideVeilPluginProvider,
  useContainerAction,
  useHaptics,
  usePluginBackendState,
  usePresentationMode,
} from '@gojona/sideveil-sdk';
import {
  AppShell,
  Button,
  Card,
  cn,
  StatusBanner,
  ThemeProvider,
  ToastProvider,
  useCopyFeedback,
  useToast,
} from '@gojona/ui';
import { Layers } from 'lucide-react';
import { useCallback, useMemo } from 'react';
import { STRATA_MANIFEST, STRATA_PLUGIN_ID } from './manifest.ts';
import { createMockStrataTransport } from './plugin/mockStrataBridge.ts';
import { QuickControlSection } from './plugin/QuickControlSection.tsx';
import { StrataHeaderActions } from './plugin/StrataHeaderActions.tsx';
import { useStrataPluginTheme } from './plugin/useStrataPluginTheme.ts';
import { useStrataQuickPanel } from './plugin/useStrataQuickPanel.ts';
import { ServicesPanel } from './shared/ServicesPanel.tsx';
import { buildSshCommand } from './shared/strataFormat.ts';

const PINNED_DEFAULT_BOUNDS = { widthDp: 344, heightDp: 520 };

function StrataWorkbench() {
  const { isNativeHost, isDark, copyText } = useStrataPluginTheme();
  const { dismiss } = useContainerAction();
  const backendState = usePluginBackendState();
  const { mode, isPinned, clickThrough, setMode, setClickThrough } = usePresentationMode('drawer');
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const backendOnline = !isNativeHost || backendState === 'online';
  const panel = useStrataQuickPanel(backendOnline, isDark);
  const active = panel.activeContainer;
  const { copyStatus, copyText: copyWithFeedback } = useCopyFeedback({ onCopy: copyText });

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

  const handleCopySsh = useCallback(async () => {
    if (!active) return;
    const cmd = buildSshCommand(active);
    const ok = await copyWithFeedback(cmd);
    pushToast(ok ? `已复制：${cmd}` : '复制失败，请手动复制', ok ? 'success' : 'danger');
  }, [active, copyWithFeedback, pushToast]);

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
        bodyClassName="flex flex-col"
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
            loading={panel.loading}
            onToggleClickThrough={handleToggleClickThrough}
            onCycleMode={handleCycleMode}
            onRefresh={() => void panel.refreshAll()}
            onDismiss={() => void dismiss()}
          />
        }
      >
        <div className="flex flex-col gap-2.5">
          {panel.containers.length > 1 ? (
            <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
              {panel.containers.map((item) => (
                <Button
                  key={item.id}
                  className="shrink-0"
                  size="sm"
                  variant={item.id === panel.selectedId ? 'pill' : 'secondary'}
                  onClick={() => panel.selectContainer(item.id)}
                >
                  {item.id}
                </Button>
              ))}
            </div>
          ) : null}

          {!backendOnline ? (
            <StatusBanner
              tone="warning"
              title="stratad 后端暂不可用"
              description="请确认 Strata 模块守护进程已启动，或打开 KernelSU WebUI 拉起服务。"
            />
          ) : panel.lastError ? (
            <StatusBanner
              tone="danger"
              title="无法连接 stratad"
              description={panel.lastError}
              action={
                <Button size="sm" variant="secondary" onClick={() => void panel.refreshAll()}>
                  重试
                </Button>
              }
            />
          ) : active ? (
            <>
              <QuickControlSection
                container={active}
                hudState={panel.hudState}
                busyAction={panel.busyAction}
                copyStatus={copyStatus}
                onLifecycle={panel.runLifecycle}
                onCopySsh={handleCopySsh}
                onSyncDns={panel.syncDns}
                onUpdateHud={panel.updateHud}
              />
              <ServicesPanel
                compact
                containerId={active.id}
                containerRunning={active.phase === 'running'}
                services={panel.services}
                busyKey={panel.busyAction}
                onAction={panel.controlService}
              />
            </>
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
