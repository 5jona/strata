import { Badge, Button, Card } from '@gojona/ui';
import { ChevronDown, ChevronUp, Terminal, Trash2, XCircle } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { TaskExitPayload, TaskOutputPayload } from '../../types/protocol.ts';

export interface ActiveToolTaskState {
  taskId: string;
  container: string;
  label: string;
  running: boolean;
  exit?: TaskExitPayload;
  entries: TaskOutputPayload[];
}

export interface TaskStreamDrawerProps {
  tasks: ActiveToolTaskState[];
  activeTaskId: string | null;
  expanded: boolean;
  onSelectTask: (taskId: string) => void;
  onToggleExpand: () => void;
  onCancel: (taskId: string) => void;
  onClearFinished: () => void;
}

/**
 * Collapsible multi-task output drawer retaining recent task history across actions.
 */
export function TaskStreamDrawer({
  tasks,
  activeTaskId,
  expanded,
  onSelectTask,
  onToggleExpand,
  onCancel,
  onClearFinished,
}: TaskStreamDrawerProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const current = tasks.find((t) => t.taskId === activeTaskId) ?? tasks[0] ?? null;
  const entryCount = current?.entries.length ?? 0;

  useEffect(() => {
    if (expanded && entryCount > 0 && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entryCount, expanded]);

  if (!current) return null;

  const statusBadge = current.running ? (
    <Badge variant="accent">执行中</Badge>
  ) : current.exit?.exitCode === 0 && !current.exit.timedOut ? (
    <Badge variant="success">已完成 (exit 0)</Badge>
  ) : (
    <Badge variant="danger">
      {current.exit?.timedOut ? '执行超时' : `退出码 ${current.exit?.exitCode ?? '?'}`}
    </Badge>
  );

  const hasFinished = tasks.some((t) => !t.running);

  return (
    <Card className="shrink-0 overflow-hidden border-[var(--border-hairline)] p-0">
      <div className="flex flex-wrap items-center justify-between gap-2 bg-[var(--surface-elevated)] px-3.5 py-2">
        <button
          type="button"
          className="flex min-w-0 items-center gap-2 text-left"
          onClick={onToggleExpand}
        >
          <Terminal className="size-3.5 shrink-0 text-[var(--accent)]" />
          <span className="truncate text-xs font-semibold text-[var(--text-primary)]">
            {current.label} · {current.container}
          </span>
          <span className="font-mono text-[11px] text-[var(--text-secondary)]">
            ({current.taskId})
          </span>
          {statusBadge}
        </button>

        <div className="flex items-center gap-1.5">
          {tasks.length > 1 && (
            <div className="flex items-center gap-1">
              {tasks.slice(0, 4).map((t) => (
                <Button
                  key={t.taskId}
                  size="sm"
                  variant={t.taskId === current.taskId ? 'pill' : 'ghost'}
                  onClick={() => onSelectTask(t.taskId)}
                >
                  {t.label.slice(0, 10)}
                </Button>
              ))}
            </div>
          )}
          {current.running && (
            <Button size="sm" variant="danger" onClick={() => onCancel(current.taskId)}>
              <XCircle className="size-3.5" />
              取消任务
            </Button>
          )}
          {hasFinished && (
            <Button
              size="icon-sm"
              variant="ghost"
              title="清除已结束任务记录"
              aria-label="清除任务日志"
              onClick={onClearFinished}
            >
              <Trash2 className="size-3.5" />
            </Button>
          )}
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={expanded ? '收起任务日志' : '展开任务日志'}
            onClick={onToggleExpand}
          >
            {expanded ? <ChevronDown className="size-4" /> : <ChevronUp className="size-4" />}
          </Button>
        </div>
      </div>

      {expanded && (
        <div
          ref={scrollRef}
          className="max-h-52 overflow-y-auto border-t border-[var(--border-hairline)] bg-[var(--surface-canvas)] p-3 font-mono text-xs leading-relaxed strata-scroll"
        >
          {current.entries.length === 0 ? (
            <span className="text-[var(--text-secondary)]">等待任务输出…</span>
          ) : (
            current.entries.map((entry) => (
              <div
                key={`${entry.taskId}-${entry.seq}`}
                className={
                  entry.stream === 'stderr'
                    ? 'whitespace-pre-wrap text-[var(--danger)]'
                    : 'whitespace-pre-wrap text-[var(--text-primary)]'
                }
              >
                {entry.chunk}
              </div>
            ))
          )}
        </div>
      )}
    </Card>
  );
}
