import { cn } from '@gojona/ui';
import { Columns2, Rows2, Terminal, X } from 'lucide-react';
import { type ReactNode, useCallback, useRef, useState } from 'react';
import {
  countLeaves,
  MAX_SPLIT_PANES,
  type SplitDirection,
  type TerminalPaneLeaf,
  type TerminalSplitBranch,
  type TerminalSplitNode,
} from './splitTree.ts';
import { formatShellLabel } from './terminalPrefs.ts';

export interface SplitPaneLayoutProps {
  root: TerminalSplitNode;
  activePaneId: string;
  onFocusPane: (paneId: string) => void;
  onSplitPane: (paneId: string, direction: SplitDirection) => void;
  onClosePane: (paneId: string) => void;
  onResizeSplit: (splitId: string, ratio: number) => void;
  renderLeaf: (leaf: TerminalPaneLeaf, focused: boolean, paneCount: number) => ReactNode;
}

interface BranchViewProps extends Omit<SplitPaneLayoutProps, 'root'> {
  node: TerminalSplitBranch;
  paneCount: number;
}

function SplitBranchView({
  node,
  paneCount,
  activePaneId,
  onFocusPane,
  onSplitPane,
  onClosePane,
  onResizeSplit,
  renderLeaf,
}: BranchViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [dragging, setDragging] = useState(false);

  const handlePointerDown = useCallback(
    (ev: React.MouseEvent<HTMLDivElement>) => {
      ev.preventDefault();
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      setDragging(true);

      const handleMove = (moveEv: MouseEvent) => {
        const ratio =
          node.direction === 'horizontal'
            ? (moveEv.clientX - rect.left) / Math.max(1, rect.width)
            : (moveEv.clientY - rect.top) / Math.max(1, rect.height);
        onResizeSplit(node.id, ratio);
      };

      const handleUp = () => {
        setDragging(false);
        window.removeEventListener('mousemove', handleMove);
        window.removeEventListener('mouseup', handleUp);
      };

      window.addEventListener('mousemove', handleMove);
      window.addEventListener('mouseup', handleUp);
    },
    [node.direction, node.id, onResizeSplit],
  );

  const isHorizontal = node.direction === 'horizontal';
  const firstPct = `${(node.ratio * 100).toFixed(2)}%`;
  const secondPct = `${((1 - node.ratio) * 100).toFixed(2)}%`;

  return (
    <div
      ref={containerRef}
      className={cn('flex h-full w-full min-h-0 min-w-0', isHorizontal ? 'flex-row' : 'flex-col')}
    >
      <div
        className="min-h-0 min-w-0 overflow-hidden"
        style={{ flexBasis: firstPct, flexGrow: 0, flexShrink: 0 }}
      >
        <SplitNodeView
          node={node.first}
          paneCount={paneCount}
          activePaneId={activePaneId}
          onFocusPane={onFocusPane}
          onSplitPane={onSplitPane}
          onClosePane={onClosePane}
          onResizeSplit={onResizeSplit}
          renderLeaf={renderLeaf}
        />
      </div>

      {/* biome-ignore lint/a11y/noStaticElementInteractions: draggable split resize handle */}
      <div
        data-dragging={dragging ? 'true' : undefined}
        onMouseDown={handlePointerDown}
        className={cn(
          'strata-split-divider',
          isHorizontal ? 'strata-split-divider-horizontal' : 'strata-split-divider-vertical',
        )}
      />

      <div
        className="min-h-0 min-w-0 overflow-hidden"
        style={{ flexBasis: secondPct, flexGrow: 1, flexShrink: 1 }}
      >
        <SplitNodeView
          node={node.second}
          paneCount={paneCount}
          activePaneId={activePaneId}
          onFocusPane={onFocusPane}
          onSplitPane={onSplitPane}
          onClosePane={onClosePane}
          onResizeSplit={onResizeSplit}
          renderLeaf={renderLeaf}
        />
      </div>
    </div>
  );
}

interface NodeViewProps extends Omit<SplitPaneLayoutProps, 'root'> {
  node: TerminalSplitNode;
  paneCount: number;
}

function SplitNodeView({
  node,
  paneCount,
  activePaneId,
  onFocusPane,
  onSplitPane,
  onClosePane,
  onResizeSplit,
  renderLeaf,
}: NodeViewProps) {
  if (node.kind === 'split') {
    return (
      <SplitBranchView
        node={node}
        paneCount={paneCount}
        activePaneId={activePaneId}
        onFocusPane={onFocusPane}
        onSplitPane={onSplitPane}
        onClosePane={onClosePane}
        onResizeSplit={onResizeSplit}
        renderLeaf={renderLeaf}
      />
    );
  }

  const focused = node.id === activePaneId;
  const canSplit = paneCount < MAX_SPLIT_PANES;

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: focuses clicked terminal pane
    <div
      onMouseDown={() => {
        if (!focused) onFocusPane(node.id);
      }}
      className={cn(
        'relative flex h-full w-full min-h-0 min-w-0 flex-col overflow-hidden',
        paneCount > 1 && focused && 'ring-1 ring-inset ring-[var(--accent)]/65',
      )}
    >
      {paneCount > 1 && (
        <div
          className={cn(
            'flex h-6 shrink-0 items-center justify-between gap-2 border-b border-[var(--border-hairline)] px-2 text-[11px]',
            focused
              ? 'bg-[var(--accent)]/12 text-[var(--text-primary)]'
              : 'bg-[var(--surface-elevated)] text-[var(--text-secondary)]',
          )}
        >
          <div className="flex min-w-0 items-center gap-1.5 font-mono">
            <Terminal
              className={cn('size-3 shrink-0', focused ? 'text-[var(--accent)]' : 'opacity-60')}
            />
            <span className="truncate">{node.title || node.containerId}</span>
            <span className="shrink-0 rounded bg-[var(--surface-card)] px-1 text-[10px] text-[var(--text-secondary)]">
              {formatShellLabel(node.shell)}
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {canSplit && (
              <>
                <button
                  type="button"
                  title="向右分屏"
                  aria-label="向右分屏"
                  onClick={(ev) => {
                    ev.stopPropagation();
                    onSplitPane(node.id, 'horizontal');
                  }}
                  className="rounded p-0.5 opacity-75 hover:bg-[var(--surface-card)] hover:text-[var(--accent)] hover:opacity-100"
                >
                  <Columns2 className="size-3" />
                </button>
                <button
                  type="button"
                  title="向下分屏"
                  aria-label="向下分屏"
                  onClick={(ev) => {
                    ev.stopPropagation();
                    onSplitPane(node.id, 'vertical');
                  }}
                  className="rounded p-0.5 opacity-75 hover:bg-[var(--surface-card)] hover:text-[var(--accent)] hover:opacity-100"
                >
                  <Rows2 className="size-3" />
                </button>
              </>
            )}
            <button
              type="button"
              title="关闭此窗格"
              aria-label="关闭此窗格"
              onClick={(ev) => {
                ev.stopPropagation();
                onClosePane(node.id);
              }}
              className="rounded p-0.5 opacity-75 hover:bg-[var(--danger)]/15 hover:text-[var(--danger)] hover:opacity-100"
            >
              <X className="size-3" />
            </button>
          </div>
        </div>
      )}
      <div className="relative min-h-0 flex-1">{renderLeaf(node, focused, paneCount)}</div>
    </div>
  );
}

/** Recursive binary split layout supporting up to 4 interactive panes per tab. */
export function SplitPaneLayout({ root, ...rest }: SplitPaneLayoutProps) {
  const paneCount = countLeaves(root);
  return <SplitNodeView node={root} paneCount={paneCount} {...rest} />;
}
