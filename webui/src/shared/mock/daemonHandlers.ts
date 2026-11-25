import { STRATA_PLUGIN_ID } from '../../manifest.ts';
import {
  STRATA_EVENTS,
  STRATA_METHODS,
  STRATA_PATHS,
  STRATA_PROTOCOL,
  type TaskOutputPayload,
} from '../../types.ts';
import { cloneConfig, type MockStrataState, type MockTaskRecord } from './state.ts';

/** Callback that emits a simulated `plugin.event` notification over the mock transport. */
export type MockEmitPluginFn = (event: string, payload: unknown) => void;

const UNHANDLED = Symbol('unhandled');

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

/** Dispatches daemon, web console, HUD, task, and event RPC methods against mock state. */
export function handleDaemonMockInvoke(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  method: string,
  params: Record<string, unknown>,
): unknown | typeof UNHANDLED {
  const { defaultId, container, config, daemonLogs, capabilities, hud, tasks, webStatus } = state;

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
    case STRATA_METHODS.DAEMON_CAPABILITIES:
      return { ...capabilities };
    case STRATA_METHODS.DAEMON_LOGS:
      return {
        logPath: STRATA_PATHS.DEFAULT_DAEMON_LOG_PATH,
        lines: [...daemonLogs],
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
      state.webToken = `demo_rotated_${Date.now().toString(36)}_0123456789abcdef`;
      return {
        token: state.webToken,
        tokenPath: webStatus.tokenPath,
        sessionsRevoked: 1,
      };
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
      return UNHANDLED;
  }
}

export { UNHANDLED };
