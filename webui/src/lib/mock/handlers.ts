import { STRATA_PLUGIN_ID } from '../../manifest.ts';
import {
  type ContainerServiceRuntime,
  STRATA_EVENTS,
  STRATA_METHODS,
  STRATA_PATHS,
  STRATA_PROTOCOL,
  type TaskOutputPayload,
} from '../../types.ts';
import {
  applyMockConfigPatch,
  cloneConfig,
  cloneSnapshot,
  type MockStrataState,
  type MockTaskRecord,
  toParamsRecord,
} from './state.ts';

/** Callback that emits a simulated `plugin.event` notification over the mock transport. */
export type MockEmitPluginFn = (event: string, payload: unknown) => void;

function handleTaskRun(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  params: Record<string, unknown>,
) {
  const { container, tasks } = state;
  const command = Array.isArray(params.command)
    ? params.command.map(String)
    : ['/bin/sh', '-c', 'echo ok'];
  const taskId = `task-${state.nextTaskSeq++}`;
  const rec: MockTaskRecord = {
    summary: {
      taskId,
      container: container.id,
      pid: 20000 + state.nextTaskSeq,
      command,
      running: true,
      startedAtMs: Date.now(),
    },
    entries: [],
  };
  tasks.push(rec);

  const emitChunk = (stream: 'stdout' | 'stderr', chunk: string, seq: number) => {
    const entry: TaskOutputPayload = {
      taskId,
      container: container.id,
      stream,
      chunk,
      seq,
    };
    rec.entries.push(entry);
    emitPlugin(STRATA_EVENTS.TASK_OUTPUT, entry);
  };

  setTimeout(() => {
    emitChunk('stdout', `[preview] $ ${command.join(' ')}`, 1);
    emitChunk('stdout', `hostname=${container.hostname} phase=${container.phase}`, 2);
    rec.summary.running = false;
    rec.summary.exitCode = 0;
    emitPlugin(STRATA_EVENTS.TASK_EXIT, {
      taskId,
      container: container.id,
      exitCode: 0,
      timedOut: false,
    });
  }, 120);

  return { taskId, container: container.id, pid: rec.summary.pid };
}

function transitionService(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  serviceId: string,
  nextStatus: 'running' | 'stopped',
  bumpRestart = false,
): ContainerServiceRuntime {
  const svc = state.services.find((s) => s.id === serviceId);
  if (!svc) throw new Error(`service not found: ${serviceId}`);
  const prevStatus = svc.status;
  svc.status = nextStatus;
  if (nextStatus === 'running') {
    svc.pid = 13_000 + Math.floor(Math.random() * 500);
    svc.uptimeMs = 1000;
  } else {
    delete svc.pid;
    svc.uptimeMs = 0;
  }
  if (bumpRestart) svc.restartCount += 1;
  const line = `[${serviceId}] ${prevStatus} -> ${nextStatus}`;
  state.serviceLogs[serviceId] = [...(state.serviceLogs[serviceId] ?? []), line];
  emitPlugin(STRATA_EVENTS.SERVICE_STATE_CHANGED, {
    container: state.container.id,
    serviceId,
    prevStatus,
    status: nextStatus,
    ...(svc.pid !== undefined ? { pid: svc.pid } : {}),
    restartCount: svc.restartCount,
  });
  return { ...svc };
}

/** Dispatches a single Strata plugin RPC method against the mutable mock state. */
export function handleMockPluginInvoke(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  method: string,
  rawParams: unknown,
): unknown {
  const { defaultId, container, config, sshKeys, dropbearLogs, hud, tasks, webStatus } = state;
  const params = toParamsRecord(rawParams);

  switch (method) {
    case STRATA_METHODS.DAEMON_PING:
      return { pong: true, pid: 4096, uptimeMs: 1_240_000 };
    case STRATA_METHODS.DAEMON_HEALTH:
      return {
        ok: true,
        pid: 4096,
        uptimeMs: 1_240_000,
        runningContainers: container.phase === 'running' ? 1 : 0,
      };
    case STRATA_METHODS.DAEMON_STATUS:
      return {
        pid: 4096,
        uid: 0,
        uptimeMs: 1_240_000,
        defaultContainer: defaultId,
        totalContainers: 1,
        runningContainers: container.phase === 'running' ? 1 : 0,
        activeTasks: tasks.filter((t) => t.summary.running).length,
        sideveilConnected: true,
      };
    case STRATA_METHODS.DAEMON_RELOAD_CONFIG:
      return {
        version: STRATA_PROTOCOL.CONFIG_SCHEMA_VERSION,
        defaultContainer: defaultId,
        sideveil: {
          enabled: true,
          hubSock: STRATA_PATHS.DEFAULT_SIDEVEIL_HUB_SOCK,
          pluginId: STRATA_PLUGIN_ID,
        },
        web: {
          enabled: webStatus.enabled,
          lanEnabled: webStatus.lanEnabled,
          port: webStatus.port,
        },
        containers: { [defaultId]: cloneConfig(config) },
      };
    case STRATA_METHODS.WEB_STATUS:
      return { ...webStatus, lanUrls: [...webStatus.lanUrls] };
    case STRATA_METHODS.WEB_CONFIGURE: {
      if (typeof params.enabled === 'boolean') webStatus.enabled = params.enabled;
      if (typeof params.lanEnabled === 'boolean') {
        webStatus.lanEnabled = params.lanEnabled;
        webStatus.listenAddr = params.lanEnabled
          ? STRATA_PROTOCOL.DEFAULT_WEB_LISTEN_LAN
          : STRATA_PROTOCOL.DEFAULT_WEB_LISTEN_LOCAL;
      }
      if (typeof params.port === 'number') webStatus.port = params.port;
      return { ...webStatus, lanUrls: [...webStatus.lanUrls] };
    }
    case STRATA_METHODS.WEB_ROTATE_TOKEN: {
      state.webToken = `rotated_${Date.now().toString(16)}_${Math.random().toString(16).slice(2, 10)}`;
      return {
        token: state.webToken,
        tokenPath: webStatus.tokenPath,
        sessionsRevoked: 1,
      };
    }
    case STRATA_METHODS.SERVICE_LIST:
      return state.services.map((s) => ({ ...s }));
    case STRATA_METHODS.SERVICE_START:
      return transitionService(state, emitPlugin, String(params.serviceId ?? ''), 'running');
    case STRATA_METHODS.SERVICE_STOP:
      return transitionService(state, emitPlugin, String(params.serviceId ?? ''), 'stopped');
    case STRATA_METHODS.SERVICE_RESTART:
      return transitionService(state, emitPlugin, String(params.serviceId ?? ''), 'running', true);
    case STRATA_METHODS.SERVICE_LOGS: {
      const sid = String(params.serviceId ?? '');
      return {
        container: container.id,
        serviceId: sid,
        lines: [...(state.serviceLogs[sid] ?? ['暂无服务日志'])],
      };
    }
    case STRATA_METHODS.CONTAINER_LIST:
      return [cloneSnapshot(container)];
    case STRATA_METHODS.CONTAINER_STATUS:
      return cloneSnapshot(container);
    case STRATA_METHODS.CONTAINER_START: {
      const previous = container.phase;
      container.phase = 'running';
      container.mounted = true;
      container.nsPid = 14820;
      container.loopDevices = ['/dev/block/loop9'];
      container.uptimeMs = 1000;
      const snap = cloneSnapshot(container);
      emitPlugin(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
        container: container.id,
        previous,
        current: container.phase,
        snapshot: snap,
      });
      return snap;
    }
    case STRATA_METHODS.CONTAINER_STOP: {
      const previous = container.phase;
      container.phase = 'stopped';
      container.mounted = false;
      delete container.nsPid;
      container.loopDevices = [];
      container.uptimeMs = 0;
      emitPlugin(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
        container: container.id,
        previous,
        current: container.phase,
        snapshot: cloneSnapshot(container),
      });
      return {
        drainedPids: 2,
        unmountedPaths: 4,
        detachedLoops: 1,
        residualMounts: 0,
        residualLoops: 0,
      };
    }
    case STRATA_METHODS.CONTAINER_RESTART: {
      container.phase = 'running';
      container.mounted = true;
      container.nsPid = 15201;
      container.uptimeMs = 500;
      container.restartCount += 1;
      return cloneSnapshot(container);
    }
    case STRATA_METHODS.CONTAINER_SYNC_DNS:
      return { container: container.id, synced: true };
    case STRATA_METHODS.CONTAINER_COMPACT:
      return {
        container: container.id,
        beforeBytes: container.disk.imageAllocatedBytes,
        afterBytes: Math.max(container.disk.imageAllocatedBytes - 8 * 1024 * 1024, 0),
        trimmedBytes: 8 * 1024 * 1024,
      };
    case STRATA_METHODS.CONTAINER_FSCK:
      return { container: container.id, exitCode: 0, output: 'rootfs.img: clean, 38102 files' };
    case STRATA_METHODS.CONTAINER_BACKUP_START: {
      const outputPath = String(params.outputPath ?? '/data/local/tmp/strata-backup.img.gz');
      setTimeout(() => {
        emitPlugin(STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE, {
          container: container.id,
          op: 'backup',
          ok: true,
          summary: `${outputPath} (342.4 MiB)`,
        });
      }, 300);
      return { started: true, container: container.id, outputPath };
    }
    case STRATA_METHODS.CONTAINER_LOGS:
      return {
        container: container.id,
        logPath: `${STRATA_PATHS.DEFAULT_LOGS_DIR}/${defaultId}-sshd.log`,
        lines: [...dropbearLogs],
      };
    case STRATA_METHODS.CONTAINER_CONFIG_GET:
      return { container: container.id, config: cloneConfig(config) };
    case STRATA_METHODS.CONTAINER_CONFIG_PATCH: {
      applyMockConfigPatch(config, container, params);
      return { container: container.id, config: cloneConfig(config) };
    }
    case STRATA_METHODS.CONTAINER_SSH_KEY_LIST:
      return { container: container.id, keys: [...sshKeys], added: 0 };
    case STRATA_METHODS.CONTAINER_SSH_KEY_ADD: {
      const incoming = String(params.publicKey ?? '').trim();
      let added = 0;
      if (incoming && !sshKeys.includes(incoming)) {
        sshKeys.push(incoming);
        added = 1;
      }
      return { container: container.id, keys: [...sshKeys], added };
    }
    case STRATA_METHODS.HUD_STATE:
      return { ...hud };
    case STRATA_METHODS.HUD_SET: {
      if (typeof params.enabled === 'boolean') hud.enabled = params.enabled;
      if (typeof params.clickThrough === 'boolean') hud.clickThrough = params.clickThrough;
      if (typeof params.isDark === 'boolean') hud.isDark = params.isDark;
      return { ...hud };
    }
    case STRATA_METHODS.TASK_LIST:
      return tasks.map((t) => ({ ...t.summary }));
    case STRATA_METHODS.TASK_LOGS: {
      const found = tasks.find((t) => t.summary.taskId === params.taskId);
      if (!found) throw new Error('task not found');
      return { summary: { ...found.summary }, entries: [...found.entries] };
    }
    case STRATA_METHODS.TASK_RUN:
      return handleTaskRun(state, emitPlugin, params);
    case STRATA_METHODS.TASK_CANCEL:
      return { cancelled: true };
    case STRATA_METHODS.EVENTS_SUBSCRIBE:
      return { subscribed: true };
    default:
      return { ok: true };
  }
}
