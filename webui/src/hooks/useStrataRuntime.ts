import { useHaptics, usePluginEvent, useSideVeilPlugin } from '@pochita/sideveil-sdk';
import { type LogEntry, useToast } from '@pochita/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { formatBytes, formatClockTime } from '../lib/strataFormat.ts';
import {
  type BackupStartResult,
  type CompactResult,
  type ContainerConfig,
  type ContainerConfigPatchParams,
  type ContainerConfigResult,
  type ContainerLogsResult,
  type ContainerStateChangedPayload,
  type ContainerStatusSnapshot,
  type DaemonSummarySnapshot,
  type FsckResult,
  type HudStateResult,
  type MaintenanceDonePayload,
  type SshKeysResult,
  STRATA_EVENTS,
  STRATA_METHODS,
  type StorageCleanupReport,
  type TaskSummary,
} from '../types.ts';
import { useStrataTasks } from './useStrataTasks.ts';

export interface StrataRuntimeApi {
  containers: ContainerStatusSnapshot[];
  selectedId: string;
  setSelectedId: (id: string) => void;
  activeContainer: ContainerStatusSnapshot | null;
  daemonSummary: DaemonSummarySnapshot | null;
  hudState: HudStateResult;
  busyAction: string | null;
  loading: boolean;
  lastError: string | null;
  tasks: TaskSummary[];
  activeTaskId: string | null;
  taskEntries: LogEntry[];
  dropbearLogs: LogEntry[];
  containerConfig: ContainerConfig | null;
  sshKeys: string[];
  refreshAll: () => Promise<void>;
  refreshConfigAndLogs: (containerId?: string) => Promise<void>;
  runLifecycle: (action: 'start' | 'stop' | 'restart') => Promise<void>;
  runMaintenance: (op: 'dns' | 'compact' | 'fsck' | 'backup') => Promise<void>;
  updateHud: (patch: { enabled?: boolean; clickThrough?: boolean }) => Promise<void>;
  runTask: (command: string[]) => Promise<void>;
  cancelTask: (taskId: string) => Promise<void>;
  selectTask: (taskId: string) => Promise<void>;
  clearTaskConsole: () => void;
  patchConfig: (patch: ContainerConfigPatchParams) => Promise<boolean>;
  addSshKey: (publicKey: string) => Promise<boolean>;
}

export function useStrataRuntime(isOnline: boolean, isDark: boolean): StrataRuntimeApi {
  const { bridge } = useSideVeilPlugin();
  const { trigger: triggerHaptic } = useHaptics();
  const { pushToast } = useToast();

  const [containers, setContainers] = useState<ContainerStatusSnapshot[]>([]);
  const [selectedId, setSelectedId] = useState<string>('main');
  const [daemonSummary, setDaemonSummary] = useState<DaemonSummarySnapshot | null>(null);
  const [hudState, setHudState] = useState<HudStateResult>({
    enabled: false,
    clickThrough: false,
    isDark: true,
    sideveilConnected: false,
  });
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [loading, setLoading] = useState<boolean>(true);
  const [lastError, setLastError] = useState<string | null>(null);

  const [dropbearLogs, setDropbearLogs] = useState<LogEntry[]>([]);
  const [containerConfig, setContainerConfig] = useState<ContainerConfig | null>(null);
  const [sshKeys, setSshKeys] = useState<string[]>([]);

  const selectedIdRef = useRef<string>(selectedId);
  selectedIdRef.current = selectedId;

  const notify = useCallback(
    (title: string, tone: 'info' | 'success' | 'danger' = 'info', description?: string) => {
      const message = description ? `${title}：${description}` : title;
      pushToast(message, tone);
      if (bridge.env === 'host') {
        bridge.toast(message);
      }
    },
    [bridge, pushToast],
  );

  const taskCtrl = useStrataTasks(selectedId, notify);
  const replaceTasks = taskCtrl.replaceTasks;

  const refreshConfigAndLogs = useCallback(
    async (targetId?: string) => {
      if (!isOnline) return;
      const cid = targetId ?? selectedIdRef.current;
      try {
        const [logsRes, cfgRes, keysRes] = await Promise.all([
          bridge.invokePlugin<ContainerLogsResult>(STRATA_METHODS.CONTAINER_LOGS, {
            container: cid,
            lines: 80,
          }),
          bridge.invokePlugin<ContainerConfigResult>(STRATA_METHODS.CONTAINER_CONFIG_GET, {
            container: cid,
          }),
          bridge.invokePlugin<SshKeysResult>(STRATA_METHODS.CONTAINER_SSH_KEY_LIST, {
            container: cid,
          }),
        ]);
        const ts = formatClockTime(Date.now());
        setDropbearLogs(
          (logsRes.lines ?? []).map((line, index) => ({
            id: `db-${index}-${line.slice(0, 12)}`,
            timestamp: ts,
            level: /error|fail|denied/i.test(line) ? 'warn' : 'info',
            message: line,
          })),
        );
        setContainerConfig(cfgRes.config);
        setSshKeys(keysRes.keys ?? []);
      } catch {
        // Secondary config/log data may fail transiently while container stops/starts.
      }
    },
    [bridge, isOnline],
  );

  const refreshAll = useCallback(async () => {
    if (!isOnline) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setLastError(null);
    try {
      const [list, summary, hud, taskList] = await Promise.all([
        bridge.invokePlugin<ContainerStatusSnapshot[]>(STRATA_METHODS.CONTAINER_LIST),
        bridge.invokePlugin<DaemonSummarySnapshot>(STRATA_METHODS.DAEMON_STATUS),
        bridge.invokePlugin<HudStateResult>(STRATA_METHODS.HUD_STATE),
        bridge.invokePlugin<TaskSummary[]>(STRATA_METHODS.TASK_LIST),
      ]);
      setContainers(list);
      setDaemonSummary(summary);
      setHudState(hud);
      replaceTasks(taskList);

      const nextSelected =
        list.find((item) => item.id === selectedIdRef.current)?.id ??
        list.find((item) => item.isDefault)?.id ??
        list[0]?.id ??
        'main';
      setSelectedId(nextSelected);
      await refreshConfigAndLogs(nextSelected);
    } catch (err) {
      setLastError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [bridge, isOnline, refreshConfigAndLogs, replaceTasks]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    if (!isOnline) return;
    void bridge
      .invokePlugin<HudStateResult>(STRATA_METHODS.HUD_SET, { isDark })
      .then(setHudState)
      .catch(() => {});
  }, [bridge, isDark, isOnline]);

  usePluginEvent<ContainerStateChangedPayload>(STRATA_EVENTS.CONTAINER_STATE_CHANGED, (payload) => {
    if (!payload?.snapshot) return;
    setContainers((prev) => {
      const exists = prev.some((c) => c.id === payload.snapshot.id);
      return exists
        ? prev.map((c) => (c.id === payload.snapshot.id ? payload.snapshot : c))
        : [...prev, payload.snapshot];
    });
  });

  usePluginEvent<MaintenanceDonePayload>(STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE, (payload) => {
    if (!payload) return;
    if (payload.ok) {
      notify('后台备份完成', 'success', payload.summary);
    } else {
      notify('后台备份失败', 'danger', payload.error ?? payload.summary);
    }
    void refreshAll();
  });

  const runLifecycle = useCallback(
    async (action: 'start' | 'stop' | 'restart') => {
      const cid = selectedIdRef.current;
      setBusyAction(action);
      void triggerHaptic(action === 'stop' ? 'heavy' : 'click');
      try {
        if (action === 'start') {
          await bridge.invokePlugin<ContainerStatusSnapshot>(
            STRATA_METHODS.CONTAINER_START,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify('容器已启动', 'success', cid);
        } else if (action === 'stop') {
          const report = await bridge.invokePlugin<StorageCleanupReport>(
            STRATA_METHODS.CONTAINER_STOP,
            { container: cid },
            { timeoutMs: 20_000 },
          );
          notify(
            '容器已停止',
            'success',
            `清理挂载=${report.unmountedPaths} 残留=${report.residualMounts}/${report.residualLoops}`,
          );
        } else {
          await bridge.invokePlugin<ContainerStatusSnapshot>(
            STRATA_METHODS.CONTAINER_RESTART,
            { container: cid },
            { timeoutMs: 25_000 },
          );
          notify('容器已重启', 'success', cid);
        }
        await refreshAll();
      } catch (err) {
        notify('操作失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, triggerHaptic],
  );

  const runMaintenance = useCallback(
    async (op: 'dns' | 'compact' | 'fsck' | 'backup') => {
      const cid = selectedIdRef.current;
      setBusyAction(op);
      void triggerHaptic('click');
      try {
        if (op === 'dns') {
          await bridge.invokePlugin(STRATA_METHODS.CONTAINER_SYNC_DNS, { container: cid });
          notify('DNS 与宿主机桥接已同步', 'success', cid);
        } else if (op === 'compact') {
          const res = await bridge.invokePlugin<CompactResult>(
            STRATA_METHODS.CONTAINER_COMPACT,
            { container: cid },
            { timeoutMs: 45_000 },
          );
          notify(
            'FITRIM 空间回收完成',
            'success',
            `${formatBytes(res.beforeBytes)} → ${formatBytes(res.afterBytes)}`,
          );
        } else if (op === 'fsck') {
          const res = await bridge.invokePlugin<FsckResult>(
            STRATA_METHODS.CONTAINER_FSCK,
            { container: cid },
            { timeoutMs: 45_000 },
          );
          notify('磁盘检查完成', res.exitCode <= 1 ? 'success' : 'danger', `exit=${res.exitCode}`);
        } else {
          const stamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
          const outputPath = `/data/local/tmp/strata-${cid}-${stamp}.img.gz`;
          const res = await bridge.invokePlugin<BackupStartResult>(
            STRATA_METHODS.CONTAINER_BACKUP_START,
            { container: cid, outputPath },
          );
          notify('后台压缩备份已启动', 'info', res.outputPath);
        }
        await refreshAll();
      } catch (err) {
        notify('运维任务失败', 'danger', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, triggerHaptic],
  );

  const updateHud = useCallback(
    async (patch: { enabled?: boolean; clickThrough?: boolean }) => {
      void triggerHaptic('tick');
      try {
        const next = await bridge.invokePlugin<HudStateResult>(STRATA_METHODS.HUD_SET, {
          ...patch,
          isDark,
        });
        setHudState(next);
      } catch (err) {
        notify('HUD 更新失败', 'danger', err instanceof Error ? err.message : String(err));
      }
    },
    [bridge, isDark, notify, triggerHaptic],
  );

  const patchConfig = useCallback(
    async (patch: ContainerConfigPatchParams): Promise<boolean> => {
      const cid = selectedIdRef.current;
      setBusyAction('config');
      void triggerHaptic('click');
      try {
        const res = await bridge.invokePlugin<ContainerConfigResult>(
          STRATA_METHODS.CONTAINER_CONFIG_PATCH,
          { ...patch, container: cid },
        );
        setContainerConfig(res.config);
        notify('配置已保存', 'success', '重启动容器后生效');
        await refreshAll();
        return true;
      } catch (err) {
        notify('保存配置失败', 'danger', err instanceof Error ? err.message : String(err));
        return false;
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, refreshAll, triggerHaptic],
  );

  const addSshKey = useCallback(
    async (publicKey: string): Promise<boolean> => {
      const cid = selectedIdRef.current;
      setBusyAction('ssh-key');
      void triggerHaptic('click');
      try {
        const res = await bridge.invokePlugin<SshKeysResult>(STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
          container: cid,
          publicKey,
        });
        setSshKeys(res.keys);
        notify(res.added > 0 ? '公钥已添加' : '公钥已存在', 'success', `共 ${res.keys.length} 条`);
        return true;
      } catch (err) {
        notify('添加公钥失败', 'danger', err instanceof Error ? err.message : String(err));
        return false;
      } finally {
        setBusyAction(null);
      }
    },
    [bridge, notify, triggerHaptic],
  );

  const activeContainer = containers.find((c) => c.id === selectedId) ?? containers[0] ?? null;

  return {
    containers,
    selectedId,
    setSelectedId,
    activeContainer,
    daemonSummary,
    hudState,
    busyAction: busyAction ?? (taskCtrl.runningTaskAction ? 'task' : null),
    loading,
    lastError,
    tasks: taskCtrl.tasks,
    activeTaskId: taskCtrl.activeTaskId,
    taskEntries: taskCtrl.taskEntries,
    dropbearLogs,
    containerConfig,
    sshKeys,
    refreshAll,
    refreshConfigAndLogs,
    runLifecycle,
    runMaintenance,
    updateHud,
    runTask: taskCtrl.runTask,
    cancelTask: taskCtrl.cancelTask,
    selectTask: taskCtrl.selectTask,
    clearTaskConsole: taskCtrl.clearTaskConsole,
    patchConfig,
    addSshKey,
  };
}
