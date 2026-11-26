import { Button, cn, EmptyState, useToast } from '@gojona/ui';
import { Terminal } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { resolveLaunchSshTarget } from '../lib/sshTarget.ts';
import '../styles/terminal.css';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile, TerminalTabSpec } from '../types/device.ts';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';
import { TerminalStatusStrip } from './TerminalOverlays.tsx';
import { TerminalPane, type TerminalPaneActions } from './TerminalPane.tsx';
import { SplitPaneLayout } from './terminal/SplitPaneLayout.tsx';
import { countLeaves, findLeaf, listLeaves, MAX_SPLIT_PANES } from './terminal/splitTree.ts';
import { TerminalSettingsModal } from './terminal/TerminalSettingsModal.tsx';
import { TerminalTabBar } from './terminal/TerminalTabBar.tsx';
import { type TerminalShellId, useTerminalPrefs } from './terminal/terminalPrefs.ts';
import { useTabSplits } from './terminal/useTabSplits.ts';

export interface TerminalWorkspaceProps {
  visible: boolean;
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  tabs: TerminalTabSpec[];
  activeTabId: string | null;
  selectedContainerId?: string;
  onSelectTab: (tabId: string) => void;
  onNewTab: (containerId: string) => void;
  onCloseTab: (tabId: string) => void;
  onBindSession: (tabId: string, sessionId: string) => void;
}

/**
 * Persistent multi-tab, split-pane xterm.js workspace with shell picker,
 * bundled Maple Mono NF CN font, 16-color themes, and modern status bar.
 */
export default function TerminalWorkspace({
  visible,
  device,
  transport,
  containers,
  tabs,
  activeTabId,
  selectedContainerId,
  onSelectTab,
  onNewTab,
  onCloseTab,
  onBindSession,
}: TerminalWorkspaceProps) {
  const { pushToast } = useToast();
  const { prefs, updatePrefs, stepFontSize, resetFontSize } = useTerminalPrefs();
  const [selectedShell, setSelectedShell] = useState<TerminalShellId>(prefs.defaultShell);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const paneActionsRef = useRef<Map<string, TerminalPaneActions>>(new Map());

  const runningContainers = useMemo(
    () => containers.filter((c) => c.phase === 'running'),
    [containers],
  );
  const preferredContainerId = useMemo(() => {
    if (selectedContainerId && runningContainers.some((c) => c.id === selectedContainerId)) {
      return selectedContainerId;
    }
    return runningContainers[0]?.id;
  }, [runningContainers, selectedContainerId]);

  const {
    splitMap,
    metricsByPane,
    requestNewTab,
    closeTab,
    splitPane,
    closePane,
    focusPane,
    resizeSplit,
    bindPaneSession,
    updatePaneTitle,
    updatePaneMetrics,
  } = useTabSplits({
    tabs,
    transport,
    defaultShell: prefs.defaultShell,
    selectedShell,
    preferredContainerId,
    onNewTab,
    onCloseTab,
    onBindSession,
  });

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const activeSplit = activeTab ? splitMap[activeTab.id] : undefined;
  const activePaneLeaf = activeSplit
    ? (findLeaf(activeSplit.root, activeSplit.activePaneId) ?? listLeaves(activeSplit.root)[0])
    : undefined;
  const activePaneMetrics = activePaneLeaf ? metricsByPane[activePaneLeaf.id] : undefined;
  const activePaneCount = activeSplit ? countLeaves(activeSplit.root) : 0;

  const handleOpenExternal = useCallback(
    (explicitContainerId?: string) => {
      const cid =
        explicitContainerId ??
        activePaneLeaf?.containerId ??
        activeTab?.containerId ??
        preferredContainerId;
      const snap = containers.find((c) => c.id === cid);
      if (!snap) return;
      void resolveLaunchSshTarget(device, snap)
        .then(async ({ target, forwardedLocalPort }) => {
          const prog = await tauriDeviceBridge.openNativeTerminal(target);
          pushToast(
            forwardedLocalPort
              ? `已建立 ADB 转发 (127.0.0.1:${forwardedLocalPort}) 并启动终端: ${prog}`
              : `已启动外部终端: ${prog}`,
            'info',
          );
        })
        .catch((err) =>
          pushToast(
            `启动系统终端失败: ${err instanceof Error ? err.message : String(err)}`,
            'warning',
          ),
        );
    },
    [activePaneLeaf, activeTab, containers, device, preferredContainerId, pushToast],
  );

  // Global shortcuts when workspace is visible (skip keys already handled inside xterm)
  useEffect(() => {
    if (!visible) return;
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.defaultPrevented || ev.isComposing || !ev.ctrlKey || !ev.shiftKey || ev.altKey) return;
      if (ev.code === 'KeyT') {
        const cid = activePaneLeaf?.containerId ?? preferredContainerId;
        if (cid) {
          ev.preventDefault();
          requestNewTab(cid);
        }
      } else if (ev.code === 'KeyW' && activeTab && activePaneLeaf) {
        ev.preventDefault();
        closePane(activeTab.id, activePaneLeaf.id);
      } else if (ev.code === 'KeyD' && activeTab && activePaneLeaf) {
        ev.preventDefault();
        splitPane(activeTab.id, activePaneLeaf.id, 'horizontal');
      } else if (ev.code === 'KeyE' && activeTab && activePaneLeaf) {
        ev.preventDefault();
        splitPane(activeTab.id, activePaneLeaf.id, 'vertical');
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [
    activePaneLeaf,
    activeTab,
    closePane,
    preferredContainerId,
    requestNewTab,
    splitPane,
    visible,
  ]);

  const registerPaneActions = useCallback((paneId: string, actions: TerminalPaneActions | null) => {
    if (actions) paneActionsRef.current.set(paneId, actions);
    else paneActionsRef.current.delete(paneId);
  }, []);

  return (
    <div className={cn('flex h-full flex-col', !visible && 'hidden')}>
      <TerminalTabBar
        tabs={tabs}
        activeTabId={activeTabId}
        splitMap={splitMap}
        runningContainers={runningContainers}
        selectedShell={selectedShell}
        onSelectShell={setSelectedShell}
        onSelectTab={onSelectTab}
        onCloseTab={closeTab}
        onNewTab={requestNewTab}
      />

      <div className="relative min-h-0 flex-1">
        {tabs.length === 0 || !transport ? (
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              icon={<Terminal className="size-9" />}
              title="尚未打开终端会话"
              description={
                runningContainers.length > 0
                  ? '选择右上角 Shell 预设并点击下方按钮，在运行中的容器内启动现代交互终端。'
                  : '请先在「容器」页面启动至少一个容器。'
              }
              action={
                preferredContainerId ? (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => requestNewTab(preferredContainerId)}
                  >
                    打开 {preferredContainerId} 终端
                  </Button>
                ) : undefined
              }
            />
          </div>
        ) : (
          tabs.map((tab) => {
            const entry = splitMap[tab.id];
            if (!entry) return null;
            const tabVisible = visible && tab.id === activeTabId;
            return (
              <div key={tab.id} className={cn('h-full w-full', !tabVisible && 'hidden')}>
                <SplitPaneLayout
                  root={entry.root}
                  activePaneId={entry.activePaneId}
                  onFocusPane={(paneId) => focusPane(tab.id, paneId)}
                  onSplitPane={(paneId, dir) => splitPane(tab.id, paneId, dir)}
                  onClosePane={(paneId) => closePane(tab.id, paneId)}
                  onResizeSplit={(splitId, ratio) => resizeSplit(tab.id, splitId, ratio)}
                  renderLeaf={(leaf, focused, paneCount) => (
                    <TerminalPane
                      key={leaf.id}
                      transport={transport}
                      containerId={leaf.containerId}
                      shell={leaf.shell}
                      visible={tabVisible}
                      focused={tabVisible && focused}
                      canSplit={paneCount < MAX_SPLIT_PANES}
                      prefs={prefs}
                      onStepFontSize={stepFontSize}
                      onResetFontSize={resetFontSize}
                      onSessionBound={(sid) => bindPaneSession(tab.id, leaf.id, sid)}
                      onTitleChange={(title) => updatePaneTitle(tab.id, leaf.id, title)}
                      onMetricsChange={(m) => updatePaneMetrics(leaf.id, m)}
                      onRegisterActions={(acts) => registerPaneActions(leaf.id, acts)}
                      onSplitRight={() => splitPane(tab.id, leaf.id, 'horizontal')}
                      onSplitDown={() => splitPane(tab.id, leaf.id, 'vertical')}
                      onNewTabShortcut={() => requestNewTab(leaf.containerId)}
                      onClosePaneShortcut={() => closePane(tab.id, leaf.id)}
                      onOpenExternalSsh={() => handleOpenExternal(leaf.containerId)}
                    />
                  )}
                />
              </div>
            );
          })
        )}
      </div>

      {activeTab && activePaneLeaf && (
        <TerminalStatusStrip
          status={activePaneMetrics?.status ?? 'connecting'}
          containerId={activePaneMetrics?.containerId ?? activePaneLeaf.containerId}
          shell={activePaneMetrics?.activeShell ?? activePaneLeaf.shell}
          cols={activePaneMetrics?.cols ?? 0}
          rows={activePaneMetrics?.rows ?? 0}
          fontSize={prefs.fontSize}
          rendererKind={activePaneMetrics?.rendererKind ?? 'dom'}
          backpressured={activePaneMetrics?.backpressured}
          canSplit={activePaneCount < MAX_SPLIT_PANES}
          canExternalSsh={runningContainers.length > 0}
          onStepFontSize={stepFontSize}
          onResetFontSize={resetFontSize}
          onSplitRight={() => splitPane(activeTab.id, activePaneLeaf.id, 'horizontal')}
          onSplitDown={() => splitPane(activeTab.id, activePaneLeaf.id, 'vertical')}
          onToggleSearch={() => paneActionsRef.current.get(activePaneLeaf.id)?.toggleSearch()}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenExternalSsh={() => handleOpenExternal(activePaneLeaf.containerId)}
        />
      )}

      <TerminalSettingsModal
        open={settingsOpen}
        prefs={prefs}
        onUpdate={(patch) => {
          updatePrefs(patch);
          if (patch.defaultShell) setSelectedShell(patch.defaultShell);
        }}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}
