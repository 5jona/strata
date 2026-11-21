import { useHaptics, usePluginEvent, useSideVeilPlugin } from '@pochita/sideveil-sdk';
import type { LogEntry } from '@pochita/ui';
import { useCallback, useRef, useState } from 'react';
import { formatClockTime } from '../lib/strataFormat.ts';
import {
  STRATA_EVENTS,
  STRATA_METHODS,
  type TaskExitPayload,
  type TaskLogsResult,
  type TaskOutputPayload,
  type TaskRunResult,
  type TaskSummary,
} from '../types.ts';

const MAX_LOG_ENTRIES = 240;

type NotifyFn = (title: string, tone?: 'info' | 'success' | 'danger', description?: string) => void;

export interface StrataTasksApi {
  tasks: TaskSummary[];
  activeTaskId: string | null;
  taskEntries: LogEntry[];
  runningTaskAction: boolean;
  replaceTasks: (next: TaskSummary[]) => void;
  runTask: (command: string[]) => Promise<void>;
  cancelTask: (taskId: string) => Promise<void>;
  selectTask: (taskId: string) => Promise<void>;
  clearTaskConsole: () => void;
}

function toTaskLogEntry(item: TaskOutputPayload): LogEntry {
  return {
    id: `${item.taskId}-${item.seq}-${item.stream}`,
    timestamp: formatClockTime(Date.now()),
    level: item.stream === 'stderr' ? 'error' : 'info',
    message: item.chunk,
  };
}

export function useStrataTasks(containerId: string, notify: NotifyFn): StrataTasksApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();

  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [activeTaskId, setActiveTaskId] = useState<string | null>(null);
  const [taskEntries, setTaskEntries] = useState<LogEntry[]>([]);
  const [runningTaskAction, setRunningTaskAction] = useState(false);

  const activeTaskIdRef = useRef<string | null>(null);
  activeTaskIdRef.current = activeTaskId;
  const containerIdRef = useRef<string>(containerId);
  containerIdRef.current = containerId;

  usePluginEvent<TaskOutputPayload>(STRATA_EVENTS.TASK_OUTPUT, (payload) => {
    if (!payload || payload.taskId !== activeTaskIdRef.current) return;
    setTaskEntries((prev) => [...prev.slice(-(MAX_LOG_ENTRIES - 1)), toTaskLogEntry(payload)]);
  });

  usePluginEvent<{ dropped?: number }>(STRATA_EVENTS.PLUGIN_EVENT_LAGGED, (payload) => {
    const dropped = typeof payload?.dropped === 'number' ? payload.dropped : 0;
    if (dropped <= 0 || !activeTaskIdRef.current) return;
    setTaskEntries((prev) => [
      ...prev.slice(-(MAX_LOG_ENTRIES - 1)),
      {
        id: `lag-${Date.now()}`,
        timestamp: formatClockTime(Date.now()),
        level: 'warn',
        message: `高频事件限流：已折叠 ${dropped} 条中间事件，可点击任务标签回放完整日志`,
      },
    ]);
  });

  usePluginEvent<TaskExitPayload>(STRATA_EVENTS.TASK_EXIT, (payload) => {
    if (!payload) return;
    setTasks((prev) =>
      prev.map((item) =>
        item.taskId === payload.taskId
          ? { ...item, running: false, exitCode: payload.exitCode }
          : item,
      ),
    );
    if (payload.taskId === activeTaskIdRef.current) {
      const level = payload.exitCode === 0 && !payload.timedOut ? 'info' : 'error';
      const detail = payload.timedOut ? '任务超时已终止' : `任务已退出 (exit=${payload.exitCode})`;
      setTaskEntries((prev) => [
        ...prev.slice(-(MAX_LOG_ENTRIES - 1)),
        {
          id: `${payload.taskId}-exit-${Date.now()}`,
          timestamp: formatClockTime(Date.now()),
          level,
          message: detail,
        },
      ]);
    }
  });

  const replaceTasks = useCallback((next: TaskSummary[]) => {
    setTasks([...next].reverse());
  }, []);

  const selectTask = useCallback(
    async (taskId: string) => {
      setActiveTaskId(taskId);
      try {
        const logs = await bridge.invokePlugin<TaskLogsResult>(STRATA_METHODS.TASK_LOGS, {
          taskId,
          limit: MAX_LOG_ENTRIES,
        });
        setTaskEntries(logs.entries.map(toTaskLogEntry));
      } catch {
        setTaskEntries([]);
      }
    },
    [bridge],
  );

  const runTask = useCallback(
    async (command: string[]) => {
      if (command.length === 0) return;
      setRunningTaskAction(true);
      void triggerHaptic('click');
      try {
        const started = await bridge.invokePlugin<TaskRunResult>(STRATA_METHODS.TASK_RUN, {
          container: containerIdRef.current,
          command,
          timeoutSecs: 120,
        });
        const newSummary: TaskSummary = {
          taskId: started.taskId,
          container: started.container,
          pid: started.pid,
          command,
          running: true,
          startedAtMs: Date.now(),
        };
        setActiveTaskId(started.taskId);
        setTasks((prev) => [newSummary, ...prev.filter((t) => t.taskId !== started.taskId)]);
        setTaskEntries([
          {
            id: `${started.taskId}-start`,
            timestamp: formatClockTime(Date.now()),
            level: 'info',
            message: `已启动 ${started.taskId} (pid=${started.pid})`,
          },
        ]);
      } catch (err) {
        notify('任务执行失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setRunningTaskAction(false);
      }
    },
    [bridge, notify, triggerHaptic],
  );

  const cancelTask = useCallback(
    async (taskId: string) => {
      void triggerHaptic('cancel');
      try {
        await bridge.invokePlugin(STRATA_METHODS.TASK_CANCEL, { taskId });
        notify('已发送中止信号', 'info', taskId);
      } catch (err) {
        notify('中止任务失败', 'danger', err instanceof Error ? err.message : String(err));
      }
    },
    [bridge, notify, triggerHaptic],
  );

  const clearTaskConsole = useCallback(() => setTaskEntries([]), []);

  return {
    tasks,
    activeTaskId,
    taskEntries,
    runningTaskAction,
    replaceTasks,
    runTask,
    cancelTask,
    selectTask,
    clearTaskConsole,
  };
}
