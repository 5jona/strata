import { MockTransport } from '@pochita/bridge';
import { STRATA_PLUGIN_ID } from '../manifest.ts';
import {
  type BindMount,
  type ContainerConfig,
  type ContainerStatusSnapshot,
  type HudStateResult,
  STRATA_EVENTS,
  STRATA_METHODS,
  type TaskOutputPayload,
  type TaskSummary,
} from '../types.ts';

interface MockTaskRecord {
  summary: TaskSummary;
  entries: TaskOutputPayload[];
}

/**
 * Creates a realistic stateful MockTransport for standalone browser preview and automated tests.
 */
export function createMockStrataTransport(): MockTransport {
  const container: ContainerStatusSnapshot = {
    id: 'main',
    isDefault: true,
    enabled: true,
    autoStart: true,
    phase: 'running',
    nsPid: 12549,
    cgroupPath: '/sys/fs/cgroup/strata/main',
    mounted: true,
    mountDir: '/data/local/strata/containers/main/mnt',
    imagePath: '/data/local/strata/containers/main/rootfs.img',
    loopDevices: ['/dev/block/loop9'],
    hostname: 'strata-main',
    sshEnabled: true,
    sshListenAddr: '0.0.0.0',
    sshPort: 2222,
    primaryIpv4: '127.0.0.1',
    hostBridge: true,
    hostRuntime: false,
    uptimeMs: 920_000,
    restartCount: 0,
    activeLeases: 0,
    disk: {
      imageExists: true,
      imageAllocatedBytes: 884 * 1024 * 1024,
      imageVirtualBytes: 32 * 1024 * 1024 * 1024,
      rootfsTotalBytes: 31 * 1024 * 1024 * 1024,
      rootfsUsedBytes: 621 * 1024 * 1024,
      rootfsAvailableBytes: 30 * 1024 * 1024 * 1024,
    },
  };

  const config: ContainerConfig = {
    enabled: true,
    autoStart: true,
    imagePath: container.imagePath,
    mountDir: container.mountDir,
    hostname: container.hostname,
    ssh: { enabled: true, listenAddr: '0.0.0.0', port: 2222, keepaliveSecs: 60 },
    oomScoreAdj: -300,
    hostBridge: true,
    hostRuntime: false,
    bindDataLocalTmp: true,
    extraCharDevs: [],
    extraMounts: [],
    syncAndroidDns: true,
    fsckOnMount: true,
  };

  const cloneConfig = (): ContainerConfig => ({
    ...config,
    ssh: { ...config.ssh },
    extraCharDevs: [...(config.extraCharDevs ?? [])],
    extraMounts: (config.extraMounts ?? []).map((m) => ({ ...m })),
  });
  const sshKeys = ['ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockPreviewKeyStrata dev@workstation'];
  const dropbearLogs = [
    '[12549] Not backgrounding',
    '[12549] Listening on 0.0.0.0:2222 (key-only)',
    '[12610] Pubkey auth succeeded for root',
  ];
  const hud: HudStateResult = {
    enabled: false,
    clickThrough: false,
    isDark: true,
    sideveilConnected: true,
  };
  const tasks: MockTaskRecord[] = [];
  let nextTaskSeq = 1;
  const transport = new MockTransport({
    namespace: 'strata-manager-preview',
    onRpc: (method) =>
      method === 'config.get'
        ? { panel: { themeId: 'coloros-glass', colorMode: 'system' } }
        : { ok: true },
    onPluginInvoke: (method, rawParams) => {
      const params = (rawParams ?? {}) as Record<string, unknown>;
      const emitPlugin = (event: string, payload: unknown) => {
        transport.emit('plugin.event', { pluginId: STRATA_PLUGIN_ID, event, payload });
      };
      switch (method) {
        case STRATA_METHODS.DAEMON_STATUS:
          return {
            pid: 4096,
            uid: 0,
            uptimeMs: 1_240_000,
            defaultContainer: 'main',
            totalContainers: 1,
            runningContainers: container.phase === 'running' ? 1 : 0,
            activeTasks: tasks.filter((t) => t.summary.running).length,
            sideveilConnected: true,
          };
        case STRATA_METHODS.CONTAINER_LIST:
          return [{ ...container, disk: { ...container.disk } }];
        case STRATA_METHODS.CONTAINER_STATUS:
          return { ...container, disk: { ...container.disk } };
        case STRATA_METHODS.CONTAINER_START: {
          const previous = container.phase;
          container.phase = 'running';
          container.mounted = true;
          container.nsPid = 14820;
          container.loopDevices = ['/dev/block/loop9'];
          container.uptimeMs = 1000;
          emitPlugin(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
            container: container.id,
            previous,
            current: container.phase,
            snapshot: { ...container },
          });
          return { ...container };
        }
        case STRATA_METHODS.CONTAINER_STOP: {
          const previous = container.phase;
          container.phase = 'stopped';
          container.mounted = false;
          container.nsPid = undefined;
          container.loopDevices = [];
          container.uptimeMs = 0;
          emitPlugin(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
            container: container.id,
            previous,
            current: container.phase,
            snapshot: { ...container },
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
          return { ...container };
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
            logPath: '/data/adb/strata/run/logs/main-sshd.log',
            lines: [...dropbearLogs],
          };
        case STRATA_METHODS.CONTAINER_CONFIG_GET:
          return { container: container.id, config: cloneConfig() };
        case STRATA_METHODS.CONTAINER_CONFIG_PATCH: {
          if (typeof params.hostname === 'string') {
            config.hostname = container.hostname = params.hostname;
          }
          if (typeof params.sshPort === 'number') {
            config.ssh.port = container.sshPort = params.sshPort;
          }
          if (typeof params.autoStart === 'boolean') {
            config.autoStart = container.autoStart = params.autoStart;
          }
          if (typeof params.hostBridge === 'boolean') {
            config.hostBridge = container.hostBridge = params.hostBridge;
          }
          if (typeof params.hostRuntime === 'boolean') {
            config.hostRuntime = container.hostRuntime = params.hostRuntime;
          }
          if (Array.isArray(params.extraCharDevs)) {
            config.extraCharDevs = params.extraCharDevs.map(String);
          }
          if (Array.isArray(params.extraMounts)) {
            config.extraMounts = (params.extraMounts as BindMount[]).map((m) => ({
              hostPath: String(m.hostPath),
              containerPath: String(m.containerPath),
              readOnly: Boolean(m.readOnly),
            }));
          }
          return { container: container.id, config: cloneConfig() };
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
        case STRATA_METHODS.TASK_RUN: {
          const command = Array.isArray(params.command)
            ? params.command.map(String)
            : ['/bin/sh', '-c', 'echo ok'];
          const taskId = `task-${nextTaskSeq++}`;
          const rec: MockTaskRecord = {
            summary: {
              taskId,
              container: container.id,
              pid: 20000 + nextTaskSeq,
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
        case STRATA_METHODS.TASK_CANCEL:
          return { cancelled: true };
        default:
          return { ok: true };
      }
    },
  });

  return transport;
}
