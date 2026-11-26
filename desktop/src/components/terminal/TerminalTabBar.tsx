import { Button, cn } from '@gojona/ui';
import { ChevronDown, Plus, Terminal, X } from 'lucide-react';
import type { TerminalTabSpec } from '../../types/device.ts';
import type { ContainerStatusSnapshot } from '../../types/protocol.ts';
import { countLeaves, findLeaf } from './splitTree.ts';
import { TERMINAL_SHELL_OPTIONS, type TerminalShellId } from './terminalPrefs.ts';
import type { TabSplitState } from './useTabSplits.ts';

export interface TerminalTabBarProps {
  tabs: TerminalTabSpec[];
  activeTabId: string | null;
  splitMap: Record<string, TabSplitState>;
  runningContainers: ContainerStatusSnapshot[];
  selectedShell: TerminalShellId;
  onSelectShell: (shell: TerminalShellId) => void;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onNewTab: (containerId: string) => void;
}

/** Top workspace tab bar with dynamic OSC titles, pane badges, container launchers, and shell picker. */
export function TerminalTabBar({
  tabs,
  activeTabId,
  splitMap,
  runningContainers,
  selectedShell,
  onSelectShell,
  onSelectTab,
  onCloseTab,
  onNewTab,
}: TerminalTabBarProps) {
  return (
    <div className="flex items-center justify-between gap-2 border-b border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-1.5">
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto strata-scroll">
        {tabs.map((tab) => {
          const active = tab.id === activeTabId;
          const splitEntry = splitMap[tab.id];
          const focusedLeaf = splitEntry
            ? findLeaf(splitEntry.root, splitEntry.activePaneId)
            : null;
          const paneCount = splitEntry ? countLeaves(splitEntry.root) : 1;
          const displayTitle = focusedLeaf?.title?.trim() || tab.title;
          return (
            <div
              key={tab.id}
              className={cn(
                'flex max-w-52 shrink-0 items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs transition-colors',
                active
                  ? 'border-[var(--accent)]/50 bg-[var(--accent)]/15 text-[var(--text-primary)]'
                  : 'border-transparent text-[var(--text-secondary)] hover:bg-[var(--surface-card)]',
              )}
            >
              <button
                type="button"
                title={displayTitle}
                className="flex min-w-0 items-center gap-1.5 font-mono"
                onClick={() => onSelectTab(tab.id)}
              >
                <Terminal className="size-3 shrink-0 text-[var(--accent)]" />
                <span className="truncate">{displayTitle}</span>
                {paneCount > 1 && (
                  <span className="shrink-0 rounded bg-[var(--surface-card)] px-1 text-[10px] text-[var(--text-secondary)]">
                    {paneCount}
                  </span>
                )}
              </button>
              <button
                type="button"
                aria-label={`关闭标签页 ${displayTitle}`}
                onClick={() => onCloseTab(tab.id)}
                className="shrink-0 opacity-70 hover:text-[var(--danger)] hover:opacity-100"
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

      {/* Shell preset selector for newly opened tabs and split panes */}
      <div className="relative flex shrink-0 items-center">
        <select
          aria-label="新建终端 Shell"
          value={selectedShell}
          onChange={(ev) => onSelectShell(ev.target.value as TerminalShellId)}
          className="h-7 appearance-none rounded-md border border-[var(--border-hairline)] bg-[var(--surface-card)] pr-6 pl-2.5 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
        >
          {TERMINAL_SHELL_OPTIONS.map((opt) => (
            <option key={opt.id} value={opt.id}>
              Shell: {opt.label}
            </option>
          ))}
        </select>
        <ChevronDown className="pointer-events-none absolute right-2 size-3 text-[var(--text-secondary)]" />
      </div>
    </div>
  );
}
