import type { TerminalShellId } from './terminalPrefs.ts';

export type SplitDirection = 'horizontal' | 'vertical';

export interface TerminalPaneLeaf {
  kind: 'leaf';
  id: string;
  containerId: string;
  shell: TerminalShellId;
  title?: string;
  sessionId?: string;
}

export interface TerminalSplitBranch {
  kind: 'split';
  id: string;
  direction: SplitDirection;
  /** Fraction [0.2, 0.8] allocated to the `first` child. */
  ratio: number;
  first: TerminalSplitNode;
  second: TerminalSplitNode;
}

export type TerminalSplitNode = TerminalPaneLeaf | TerminalSplitBranch;

export const MAX_SPLIT_PANES = 4;

let paneSeq = 1;

export function createPaneLeaf(
  containerId: string,
  shell: TerminalShellId = 'auto',
  explicitId?: string,
): TerminalPaneLeaf {
  const id = explicitId ?? `pane-${Date.now().toString(36)}-${paneSeq++}`;
  return { kind: 'leaf', id, containerId, shell };
}

export function countLeaves(node: TerminalSplitNode): number {
  if (node.kind === 'leaf') return 1;
  return countLeaves(node.first) + countLeaves(node.second);
}

export function listLeaves(node: TerminalSplitNode): TerminalPaneLeaf[] {
  if (node.kind === 'leaf') return [node];
  return [...listLeaves(node.first), ...listLeaves(node.second)];
}

export function findLeaf(node: TerminalSplitNode, paneId: string): TerminalPaneLeaf | null {
  if (node.kind === 'leaf') return node.id === paneId ? node : null;
  return findLeaf(node.first, paneId) ?? findLeaf(node.second, paneId);
}

/** Splits the target leaf into a branch containing the original leaf and `newLeaf`. */
export function splitPaneInTree(
  node: TerminalSplitNode,
  targetPaneId: string,
  direction: SplitDirection,
  newLeaf: TerminalPaneLeaf,
): TerminalSplitNode {
  if (countLeaves(node) >= MAX_SPLIT_PANES) return node;
  return splitRecursive(node, targetPaneId, direction, newLeaf);
}

function splitRecursive(
  node: TerminalSplitNode,
  targetPaneId: string,
  direction: SplitDirection,
  newLeaf: TerminalPaneLeaf,
): TerminalSplitNode {
  if (node.kind === 'leaf') {
    if (node.id !== targetPaneId) return node;
    return {
      kind: 'split',
      id: `split-${Date.now().toString(36)}-${paneSeq++}`,
      direction,
      ratio: 0.5,
      first: node,
      second: newLeaf,
    };
  }
  const nextFirst = splitRecursive(node.first, targetPaneId, direction, newLeaf);
  if (nextFirst !== node.first) return { ...node, first: nextFirst };
  const nextSecond = splitRecursive(node.second, targetPaneId, direction, newLeaf);
  if (nextSecond !== node.second) return { ...node, second: nextSecond };
  return node;
}

/** Removes a leaf from the split tree, returning `null` if the last leaf is removed. */
export function removePaneFromTree(
  node: TerminalSplitNode,
  targetPaneId: string,
): TerminalSplitNode | null {
  if (node.kind === 'leaf') {
    return node.id === targetPaneId ? null : node;
  }
  const nextFirst = removePaneFromTree(node.first, targetPaneId);
  if (nextFirst === null) return node.second;
  const nextSecond = removePaneFromTree(node.second, targetPaneId);
  if (nextSecond === null) return node.first;
  if (nextFirst === node.first && nextSecond === node.second) return node;
  return { ...node, first: nextFirst, second: nextSecond };
}

/** Applies a shallow patch to a single leaf node inside the split tree. */
export function updateLeafInTree(
  node: TerminalSplitNode,
  paneId: string,
  patch: Partial<Pick<TerminalPaneLeaf, 'title' | 'sessionId' | 'shell'>>,
): TerminalSplitNode {
  if (node.kind === 'leaf') {
    if (node.id !== paneId) return node;
    if (
      (patch.title === undefined || patch.title === node.title) &&
      (patch.sessionId === undefined || patch.sessionId === node.sessionId) &&
      (patch.shell === undefined || patch.shell === node.shell)
    ) {
      return node;
    }
    return { ...node, ...patch };
  }
  const nextFirst = updateLeafInTree(node.first, paneId, patch);
  const nextSecond = updateLeafInTree(node.second, paneId, patch);
  if (nextFirst === node.first && nextSecond === node.second) return node;
  return { ...node, first: nextFirst, second: nextSecond };
}

/** Updates a split branch ratio clamped between 20% and 80%. */
export function updateSplitRatio(
  node: TerminalSplitNode,
  splitId: string,
  ratio: number,
): TerminalSplitNode {
  if (node.kind === 'leaf') return node;
  const clamped = Math.max(0.2, Math.min(0.8, ratio));
  if (node.id === splitId) {
    return { ...node, ratio: clamped };
  }
  const nextFirst = updateSplitRatio(node.first, splitId, clamped);
  const nextSecond = updateSplitRatio(node.second, splitId, clamped);
  if (nextFirst === node.first && nextSecond === node.second) return node;
  return { ...node, first: nextFirst, second: nextSecond };
}

/** Finds an adjacent leaf ID when cycling focus or closing a pane. */
export function getAdjacentPaneId(
  node: TerminalSplitNode,
  currentPaneId: string,
  delta: 1 | -1 = 1,
): string | null {
  const leaves = listLeaves(node);
  if (leaves.length === 0) return null;
  const idx = leaves.findIndex((leaf) => leaf.id === currentPaneId);
  if (idx < 0) return leaves[0]?.id ?? null;
  const nextIdx = (idx + delta + leaves.length) % leaves.length;
  return leaves[nextIdx]?.id ?? null;
}
