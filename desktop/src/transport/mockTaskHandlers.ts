import type { TaskOutputPayload, TaskRunResult, TaskSummary } from '../types/protocol.ts';
import { STRATA_EVENTS, StrataRpcError } from '../types/rpc.ts';
import type { MockRuntimeContext } from './mockHandlers.ts';

export interface MockTaskRecord {
  summary: TaskSummary;
  entries: TaskOutputPayload[];
  cancelled: boolean;
}

export function launchMockTask(
  ctx: MockRuntimeContext,
  tasks: Map<string, MockTaskRecord>,
  container: string,
  command: string[],
  lines: string[],
  reason: string,
  onComplete: () => void,
): TaskRunResult {
  const taskId = `task-${Date.now().toString(36)}-${Math.floor(Math.random() * 999)}`;
  const record: MockTaskRecord = {
    summary: {
      taskId,
      container,
      pid: 24_100 + tasks.size,
      command,
      running: true,
      startedAtMs: Date.now(),
    },
    entries: [],
    cancelled: false,
  };
  tasks.set(taskId, record);

  lines.forEach((line, idx) => {
    ctx.schedule(140 * (idx + 1), () => {
      if (record.cancelled) return;
      const entry: TaskOutputPayload = {
        taskId,
        container,
        stream: 'stdout',
        chunk: `${line}\n`,
        seq: idx + 1,
      };
      record.entries.push(entry);
      ctx.emit(STRATA_EVENTS.TASK_OUTPUT, entry);
    });
  });

  ctx.schedule(140 * (lines.length + 1), () => {
    if (record.cancelled) return;
    onComplete();
    record.summary.running = false;
    record.summary.exitCode = 0;
    ctx.emit(STRATA_EVENTS.TASK_EXIT, { taskId, container, exitCode: 0, timedOut: false });
    ctx.emit(STRATA_EVENTS.TOOLS_STATE_CHANGED, { container, reason, taskId });
  });

  return { taskId, container, pid: record.summary.pid };
}

export function handleMockTaskRpc(
  ctx: MockRuntimeContext,
  tasks: Map<string, MockTaskRecord>,
  method: string,
  rawParams: unknown,
): unknown {
  const p = (rawParams ?? {}) as Record<string, unknown>;
  switch (method) {
    case 'task.list':
      return Array.from(tasks.values()).map((t) => t.summary);
    case 'task.logs': {
      const rec = tasks.get(String(p.taskId));
      if (!rec) throw new StrataRpcError(-32025, '任务不存在');
      return { summary: rec.summary, entries: rec.entries };
    }
    case 'task.cancel': {
      const rec = tasks.get(String(p.taskId));
      if (!rec?.summary.running) return { cancelled: false };
      rec.cancelled = true;
      rec.summary.running = false;
      rec.summary.exitCode = 130;
      ctx.emit(STRATA_EVENTS.TASK_EXIT, {
        taskId: rec.summary.taskId,
        container: rec.summary.container,
        exitCode: 130,
        signal: 15,
        timedOut: false,
      });
      return { cancelled: true };
    }
    case 'task.run': {
      const container = String(p.container ?? 'main');
      const command = Array.isArray(p.command) ? p.command.map(String) : ['sh'];
      return launchMockTask(
        ctx,
        tasks,
        container,
        command,
        [`$ ${command.join(' ')}`, 'ok'],
        'task_completed',
        () => {},
      );
    }
    default:
      return undefined;
  }
}
