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
  cn,
  ThemeProvider,
  ToastProvider,
  useCopyFeedback,
  useToast,
} from '@gojona/ui';
import { Layers } from 'lucide-react';
import { useCallback, useMemo, useRef } from 'react';
import { STRATA_MANIFEST, STRATA_PLUGIN_ID } from './manifest.ts';
import { QuickControlSection } from './plugin/QuickControlSection.tsx';
import { StrataHeaderActions } from './plugin/StrataHeaderActions.tsx';
import {
  StrataPanelFallbackCard,
  StrataPanelPermissionBanner,
} from './plugin/StrataPanelStateCards.tsx';
import { useStrataPluginTheme } from './plugin/useStrataPluginTheme.ts';
import { useStrataQuickPanel } from './plugin/useStrataQuickPanel.ts';
import { ServicesPanel } from './shared/ServicesPanel.tsx';
import { buildSshCommand } from './shared/strataFormat.ts';
import { useLazyMockTransport } from './shared/useLazyMockTransport.ts';

const PINNED_DEFAULT_BOUNDS = { widthDp: 344, heightDp: 520 };

function StrataWorkbench() {
  const { isNativeHost, isDark, copyTextDetailed } = useStrataPluginTheme();
  const { dismiss } = useContainerAction();
  const backendState = usePluginBackendState();
  const { mode, isPinned, clickThrough, setMode, setClickThrough } = usePresentationMode('drawer');
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const backendOnline = !isNativeHost || backendState === 'online';
  const panel = useStrataQuickPanel(backendOnline, isDark);
  const active = panel.activeContainer;
  const lastCopyDeniedRef = useRef(false);
  const { copyStatus, copyText: copyWithFeedback } = useCopyFeedback({
    onCopy: async (text) => {
      const res = await copyTextDetailed(text);
      lastCopyDeniedRef.current = res.permissionDenied;
      return res.ok;
    },
  });

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
    if (ok) {
      panel.clearPermissionNotice();
      pushToast(`已复制：${cmd}`, 'success');
    } else if (lastCopyDeniedRef.current) {
      panel.showPermissionGuide('剪贴板权限尚未开启');
    } else {
      pushToast('复制失败，请手动复制', 'danger');
    }
  }, [active, copyWithFeedback, panel, pushToast]);

  return (
    <AppShell
      variant="drawer"
      title="Strata 容器"
      bodyClassName="flex flex-col"
      className={
        isNativeHost
          ? undefined
          : cn(
              'mx-auto my-2 h-[calc(100dvh-1rem)] rounded-[var(--radius-panel)]',
              mode === 'wide' ? 'max-w-[680px]' : 'max-w-[380px]',
            )
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

        <StrataPanelPermissionBanner
          notice={panel.permissionNotice}
          onDismiss={panel.clearPermissionNotice}
        />

        {backendOnline && !panel.lastError && active ? (
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
          <StrataPanelFallbackCard
            backendOnline={backendOnline}
            lastError={panel.lastError}
            loading={panel.loading}
            onRetry={() => void panel.refreshAll()}
          />
        )}
      </div>
    </AppShell>
  );
}

const loadPluginMock = () =>
  import('./plugin/mockStrataBridge.ts').then((m) => m.createMockStrataTransport());

/** Root SideVeil plugin component wiring bridge, theme, and toast providers for Strata Manager. */
export function StrataManagerApp() {
  const { ready, mockTransport } = useLazyMockTransport(loadPluginMock, false);

  const bridgeOptions = useMemo<BridgeOptions>(
    () => ({
      allowUrlOverride: false,
      allowedOrigins: [`https://${STRATA_PLUGIN_ID}.plugins.sideveil.local`],
      mockNamespace: 'strata-manager',
      ...(mockTransport ? { mockTransport } : {}),
    }),
    [mockTransport],
  );

  if (!ready) return null;

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
