import { Button, cn, EmptyState, useToast } from '@gojona/ui';
import { ExternalLink, Plus, Terminal, X } from 'lucide-react';
import { resolveLaunchSshTarget } from '../lib/sshTarget.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile, TerminalTabSpec } from '../types/device.ts';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';
import { TerminalPane } from './TerminalPane.tsx';

export interface TerminalWorkspaceProps {
  visible: boolean;
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  containers: ContainerStatusSnapshot[];
  tabs: TerminalTabSpec[];
  activeTabId: string | null;
  onSelectTab: (tabId: string) => void;
  onNewTab: (containerId: string) => void;
  onCloseTab: (tabId: string) => void;
  onBindSession: (tabId: string, sessionId: string) => void;
}

/**
 * Persistent multi-tab xterm.js workspace (lazy-loaded chunk so terminal assets stay off initial load).
 */
export default function TerminalWorkspace({
  visible,
  device,
  transport,
  containers,
  tabs,
  activeTabId,
  onSelectTab,
  onNewTab,
  onCloseTab,
  onBindSession,
}: TerminalWorkspaceProps) {
  const { pushToast } = useToast();
  const runningContainers = containers.filter((c) => c.phase === 'running');
  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;

  const handleOpenExternal = () => {
    const cid = activeTab?.containerId ?? runningContainers[0]?.id;
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
  };

  return (
    <div className={cn('flex h-full flex-col', !visible && 'hidden')}>
      {/* Tab Bar */}
      <div className="flex items-center justify-between gap-2 border-b border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto strata-scroll">
          {tabs.map((tab) => {
            const active = tab.id === activeTabId;
            return (
              <div
                key={tab.id}
                className={cn(
                  'flex shrink-0 items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs transition-colors',
                  active
                    ? 'border-[var(--accent)]/50 bg-[var(--accent)]/15 text-[var(--text-primary)]'
                    : 'border-transparent text-[var(--text-secondary)] hover:bg-[var(--surface-card)]',
                )}
              >
                <button
                  type="button"
                  className="flex items-center gap-1.5 font-mono"
                  onClick={() => onSelectTab(tab.id)}
                >
                  <Terminal className="size-3 text-[var(--accent)]" />
                  <span>{tab.title}</span>
                </button>
                <button
                  type="button"
                  aria-label={`关闭标签页 ${tab.title}`}
                  onClick={() => onCloseTab(tab.id)}
                  className="opacity-70 hover:text-[var(--danger)] hover:opacity-100"
                >
                  <X className="size-3" />
                </button>
              </div>
            );
          })}

          {runningContainers.map((c) => (
            <Button
              key={`new-${c.id}`}
              size="sm"
              variant="ghost"
              className="shrink-0"
              onClick={() => onNewTab(c.id)}
            >
              <Plus className="size-3.5" />
              {c.id}
            </Button>
          ))}
        </div>

        <Button
          size="sm"
          variant="secondary"
          disabled={runningContainers.length === 0}
          onClick={handleOpenExternal}
        >
          <ExternalLink className="size-3.5" />
          外部终端 SSH
        </Button>
      </div>

      {/* Mounted Terminal Panes (kept alive across tab/route switches) */}
      <div className="relative min-h-0 flex-1">
        {tabs.length === 0 || !transport ? (
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              icon={<Terminal className="size-9" />}
              title="尚未打开终端会话"
              description={
                runningContainers.length > 0
                  ? '点击下方按钮在运行中的容器内建立交互式 WebSocket PTY 会话。'
                  : '请先在「容器」页面启动至少一个容器。'
              }
              action={
                runningContainers[0] ? (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => onNewTab(runningContainers[0].id)}
                  >
                    打开 {runningContainers[0].id} 终端
                  </Button>
                ) : undefined
              }
            />
          </div>
        ) : (
          tabs.map((tab) => (
            <TerminalPane
              key={tab.id}
              transport={transport}
              containerId={tab.containerId}
              visible={visible && tab.id === activeTabId}
              onSessionBound={(sid) => onBindSession(tab.id, sid)}
            />
          ))
        )}
      </div>
    </div>
  );
}
