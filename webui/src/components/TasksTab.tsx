import { Badge, Button, Card, Input, LogConsole, type LogEntry } from '@pochita/ui';
import { Play, Square, Terminal } from 'lucide-react';
import { useState } from 'react';
import { buildTaskCommand, QUICK_TASK_PRESETS, type TaskRunnerMode } from '../lib/strataFormat.ts';
import type { TaskSummary } from '../types.ts';

/** Props accepted by `TasksTab`. */
export interface TasksTabProps {
  containerRunning: boolean;
  tasks: readonly TaskSummary[];
  activeTaskId: string | null;
  entries: readonly LogEntry[];
  busy: boolean;
  onRunTask: (command: string[]) => Promise<void>;
  onCancelTask: (taskId: string) => Promise<void>;
  onSelectTask: (taskId: string) => Promise<void>;
  onClearConsole: () => void;
  onCopyLog: (text: string) => Promise<boolean>;
  onImeFocusChange: (focused: boolean) => void;
}

/** Headless task runner tab supporting quick presets, shell/Python execution, and streaming logs. */
export function TasksTab({
  containerRunning,
  tasks,
  activeTaskId,
  entries,
  busy,
  onRunTask,
  onCancelTask,
  onSelectTask,
  onClearConsole,
  onCopyLog,
  onImeFocusChange,
}: TasksTabProps) {
  const [mode, setMode] = useState<TaskRunnerMode>('shell');
  const [input, setInput] = useState('uname -a && python3 --version');

  const activeTask = tasks.find((t) => t.taskId === activeTaskId) ?? null;

  const handleSubmit = async () => {
    const command = buildTaskCommand(mode, input);
    if (command.length === 0) return;
    await onRunTask(command);
  };

  return (
    <div className="flex flex-col gap-2.5">
      <Card className="flex flex-col gap-2.5 p-3">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <Terminal className="size-3.5 shrink-0 text-[var(--accent)]" />
            <span className="truncate text-xs font-medium text-[var(--text-primary)]">
              端侧脚本执行器
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <Button
              size="sm"
              variant={mode === 'shell' ? 'pill' : 'ghost'}
              aria-label="切换到 Shell 模式"
              onClick={() => setMode('shell')}
            >
              Shell
            </Button>
            <Button
              size="sm"
              variant={mode === 'python' ? 'pill' : 'ghost'}
              aria-label="切换到 Python 3 模式"
              onClick={() => setMode('python')}
            >
              Python 3
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap gap-1.5">
          {QUICK_TASK_PRESETS.map((preset) => (
            <Button
              key={preset.id}
              className="shrink-0"
              size="sm"
              variant="secondary"
              disabled={!containerRunning || busy}
              aria-label={`执行预设：${preset.label}`}
              onClick={() => {
                setMode(preset.mode);
                setInput(preset.script);
                void onRunTask(buildTaskCommand(preset.mode, preset.script));
              }}
            >
              {preset.label}
            </Button>
          ))}
        </div>

        <div className="flex items-center gap-1.5">
          <Input
            mono
            value={input}
            placeholder={mode === 'python' ? '输入 Python 表达式或脚本…' : '输入容器内 Shell 命令…'}
            aria-label="容器命令输入框"
            disabled={!containerRunning}
            onFocus={() => onImeFocusChange(true)}
            onBlur={() => onImeFocusChange(false)}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                void handleSubmit();
              }
            }}
          />
          {activeTask?.running ? (
            <Button
              className="shrink-0"
              size="sm"
              variant="danger"
              aria-label="中止当前任务"
              onClick={() => void onCancelTask(activeTask.taskId)}
            >
              <Square className="size-3.5" />
              中止
            </Button>
          ) : (
            <Button
              className="shrink-0"
              size="sm"
              variant="primary"
              disabled={!containerRunning || busy || input.trim().length === 0}
              aria-label="执行容器任务"
              onClick={() => void handleSubmit()}
            >
              <Play className="size-3.5" />
              运行
            </Button>
          )}
        </div>

        {!containerRunning ? (
          <p className="text-[11px] text-[var(--warning)]">
            容器当前未运行，请先在「总览」中启动容器后再执行实时任务。
          </p>
        ) : null}
      </Card>

      {tasks.length > 0 ? (
        <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
          {tasks.slice(0, 6).map((task) => {
            const selected = task.taskId === activeTaskId;
            return (
              <Button
                key={task.taskId}
                className="shrink-0"
                size="sm"
                variant={selected ? 'pill' : 'secondary'}
                aria-label={`查看任务 ${task.taskId}`}
                onClick={() => void onSelectTask(task.taskId)}
              >
                <span className="font-mono">{task.taskId}</span>
                <Badge
                  className="shrink-0"
                  variant={task.running ? 'accent' : task.exitCode === 0 ? 'success' : 'danger'}
                  dot
                  pulse={task.running}
                >
                  {task.running ? '运行中' : `退出 ${task.exitCode ?? '-'}`}
                </Badge>
              </Button>
            );
          })}
        </div>
      ) : null}

      <LogConsole
        title={activeTask ? `任务输出 · ${activeTask.taskId}` : '实时任务输出'}
        entries={entries}
        searchable
        filterable
        onClear={onClearConsole}
        onCopy={onCopyLog}
        maxHeightClassName="max-h-60"
      />
    </div>
  );
}
