import { useCallback, useEffect, useRef, useState } from 'react';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type { TerminalTabSpec } from '../../types/device.ts';
import type { TerminalPaneMetrics } from '../TerminalPane.tsx';
import {
  countLeaves,
  createPaneLeaf,
  findLeaf,
  getAdjacentPaneId,
  listLeaves,
  MAX_SPLIT_PANES,
  removePaneFromTree,
  type SplitDirection,
  splitPaneInTree,
  type TerminalSplitNode,
  updateLeafInTree,
  updateSplitRatio,
} from './splitTree.ts';
import type { TerminalShellId } from './terminalPrefs.ts';

export interface TabSplitState {
  root: TerminalSplitNode;
  activePaneId: string;
}

export interface UseTabSplitsOptions {
  tabs: TerminalTabSpec[];
  transport: StrataTransport | null;
  defaultShell: TerminalShellId;
  selectedShell: TerminalShellId;
  preferredContainerId?: string;
  onNewTab: (containerId: string) => void;
  onCloseTab: (tabId: string) => void;
  onBindSession: (tabId: string, sessionId: string) => void;
}

/** Manages per-tab binary split trees, pane metrics, PTY cleanup, and shell assignment. */
export function useTabSplits({
  tabs,
  transport,
  defaultShell,
  selectedShell,
  preferredContainerId,
  onNewTab,
  onCloseTab,
  onBindSession,
}: UseTabSplitsOptions) {
  const [splitMap, setSplitMap] = useState<Record<string, TabSplitState>>({});
  const [metricsByPane, setMetricsByPane] = useState<Record<string, TerminalPaneMetrics>>({});
  const pendingShellRef = useRef<TerminalShellId | null>(null);
  const transportRef = useRef(transport);
  transportRef.current = transport;

  // Keep split trees aligned with parent tab list
  useEffect(() => {
    setSplitMap((prev) => {
      const activeIds = new Set(tabs.map((t) => t.id));
      let changed = false;
      const next: Record<string, TabSplitState> = {};

      for (const [id, entry] of Object.entries(prev)) {
        if (activeIds.has(id)) next[id] = entry;
        else changed = true;
      }

      for (const tab of tabs) {
        if (!next[tab.id]) {
          const shell = pendingShellRef.current ?? defaultShell;
          pendingShellRef.current = null;
          const root = createPaneLeaf(tab.containerId, shell, `${tab.id}:root`);
          next[tab.id] = { root, activePaneId: root.id };
          changed = true;
        }
      }

      return changed ? next : prev;
    });
  }, [defaultShell, tabs]);

  const requestNewTab = useCallback(
    (containerId: string, shellOverride?: TerminalShellId) => {
      pendingShellRef.current = shellOverride ?? selectedShell;
      onNewTab(containerId);
    },
    [onNewTab, selectedShell],
  );

  const closeTab = useCallback(
    (tabId: string) => {
      const entry = splitMap[tabId];
      if (entry && transportRef.current) {
        for (const leaf of listLeaves(entry.root)) {
          if (leaf.id !== `${tabId}:root` && leaf.sessionId) {
            void transportRef.current.killPty(leaf.sessionId).catch(() => {});
          }
        }
      }
      onCloseTab(tabId);
    },
    [onCloseTab, splitMap],
  );

  const splitPane = useCallback(
    (tabId: string, paneId: string, direction: SplitDirection) => {
      setSplitMap((prev) => {
        const entry = prev[tabId];
        if (!entry || countLeaves(entry.root) >= MAX_SPLIT_PANES) return prev;
        const source = findLeaf(entry.root, paneId);
        const containerId = source?.containerId ?? preferredContainerId ?? 'main';
        const newLeaf = createPaneLeaf(containerId, selectedShell);
        const nextRoot = splitPaneInTree(entry.root, paneId, direction, newLeaf);
        return { ...prev, [tabId]: { root: nextRoot, activePaneId: newLeaf.id } };
      });
    },
    [preferredContainerId, selectedShell],
  );

  const closePane = useCallback(
    (tabId: string, paneId: string) => {
      const entry = splitMap[tabId];
      if (!entry) return;
      if (listLeaves(entry.root).length <= 1) {
        closeTab(tabId);
        return;
      }
      const closing = findLeaf(entry.root, paneId);
      if (closing?.sessionId && transportRef.current) {
        void transportRef.current.killPty(closing.sessionId).catch(() => {});
      }
      const fallbackId = getAdjacentPaneId(entry.root, paneId, -1);
      const nextRoot = removePaneFromTree(entry.root, paneId);
      if (!nextRoot) {
        closeTab(tabId);
        return;
      }
      setSplitMap((prev) => ({
        ...prev,
        [tabId]: {
          root: nextRoot,
          activePaneId:
            fallbackId && findLeaf(nextRoot, fallbackId)
              ? fallbackId
              : (listLeaves(nextRoot)[0]?.id ?? ''),
        },
      }));
      setMetricsByPane((prev) => {
        if (!(paneId in prev)) return prev;
        const { [paneId]: _removed, ...rest } = prev;
        return rest;
      });
    },
    [closeTab, splitMap],
  );

  const updatePaneMetrics = useCallback((paneId: string, m: TerminalPaneMetrics) => {
    setMetricsByPane((prev) => {
      const cur = prev[paneId];
      if (
        cur &&
        cur.status === m.status &&
        cur.cols === m.cols &&
        cur.rows === m.rows &&
        cur.activeShell === m.activeShell &&
        cur.rendererKind === m.rendererKind &&
        cur.backpressured === m.backpressured
      ) {
        return prev;
      }
      return { ...prev, [paneId]: m };
    });
  }, []);

  const focusPane = useCallback((tabId: string, paneId: string) => {
    setSplitMap((prev) =>
      prev[tabId] && prev[tabId].activePaneId !== paneId
        ? { ...prev, [tabId]: { ...prev[tabId], activePaneId: paneId } }
        : prev,
    );
  }, []);

  const resizeSplit = useCallback((tabId: string, splitId: string, ratio: number) => {
    setSplitMap((prev) =>
      prev[tabId]
        ? {
            ...prev,
            [tabId]: { ...prev[tabId], root: updateSplitRatio(prev[tabId].root, splitId, ratio) },
          }
        : prev,
    );
  }, []);

  const bindPaneSession = useCallback(
    (tabId: string, paneId: string, sessionId: string) => {
      if (paneId === `${tabId}:root`) onBindSession(tabId, sessionId);
      setSplitMap((prev) =>
        prev[tabId]
          ? {
              ...prev,
              [tabId]: {
                ...prev[tabId],
                root: updateLeafInTree(prev[tabId].root, paneId, { sessionId }),
              },
            }
          : prev,
      );
    },
    [onBindSession],
  );

  const updatePaneTitle = useCallback((tabId: string, paneId: string, title: string) => {
    setSplitMap((prev) =>
      prev[tabId]
        ? {
            ...prev,
            [tabId]: {
              ...prev[tabId],
              root: updateLeafInTree(prev[tabId].root, paneId, { title }),
            },
          }
        : prev,
    );
  }, []);

  return {
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
  };
}
