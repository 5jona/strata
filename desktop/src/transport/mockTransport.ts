import type { ContainerConfig, ContainerJobProgressPayload } from '../types/protocol.ts';
import {
  STRATA_EVENTS,
  STRATA_METHODS,
  type StrataDesktopEventMap,
  type StrataDesktopEventName,
  type StrataDesktopMethod,
  type StrataDesktopRpcMap,
  type StrataEventEnvelope,
  StrataRpcError,
} from '../types/rpc.ts';
import type {
  EventListener,
  ImportUploadOptions,
  PtyConnectionHandlers,
  PtySessionHandle,
  StrataTransport,
} from './httpWsTransport.ts';
import {
  createMockDatasetForDevice,
  MOCK_DISTRO_CATALOG,
  type MockDeviceDataset,
} from './mockData.ts';
import {
  cloneMockContainer,
  deleteMockContainer,
  type MockRuntimeContext,
  patchMockContainerConfig,
  registerNewMockContainer,
  requireMockContainer,
  setMockContainerPhase,
  setMockServiceStatus,
  startMockCreateJob,
  upsertMockService,
} from './mockHandlers.ts';
import { createMockPtySession } from './mockPty.ts';

/**
 * Full stateful in-memory Strata transport for browser preview and unit tests.
 */
export class MockStrataTransport implements StrataTransport {
  readonly mode = 'mock' as const;
  readonly baseUrl: string;
  private readonly dataset: MockDeviceDataset;
  private readonly listeners = new Set<EventListener>();
  private readonly jobs = new Map<string, ContainerJobProgressPayload>();
  private readonly timers = new Set<ReturnType<typeof setTimeout>>();

  constructor(deviceId = 'mock-tablet') {
    this.baseUrl = `mock://${deviceId}`;
    this.dataset = createMockDatasetForDevice(deviceId);
  }

  private ctx(): MockRuntimeContext {
    return {
      dataset: this.dataset,
      jobs: this.jobs,
      emit: (event, payload) => this.emit(event, payload),
      schedule: (delay, fn) => this.schedule(delay, fn),
    };
  }

  private emit<E extends StrataDesktopEventName>(
    event: E,
    payload: StrataDesktopEventMap[E],
  ): void {
    const envelope = { event, payload } as StrataEventEnvelope;
    for (const listener of this.listeners) listener(envelope);
  }

  private schedule(delayMs: number, fn: () => void): void {
    const t = setTimeout(() => {
      this.timers.delete(t);
      fn();
    }, delayMs);
    this.timers.add(t);
  }

  async invoke<M extends StrataDesktopMethod>(
    method: M,
    params?: StrataDesktopRpcMap[M]['params'],
  ): Promise<StrataDesktopRpcMap[M]['result']> {
    const res = this.dispatch(method, params);
    return structuredClone(res) as StrataDesktopRpcMap[M]['result'];
  }

  private dispatch(method: StrataDesktopMethod, rawParams: unknown): unknown {
    const p = (rawParams ?? {}) as Record<string, unknown>;
    const ctx = this.ctx();
    switch (method) {
      case STRATA_METHODS.DAEMON_PING:
        return { pong: true, pid: 1420, uptimeMs: 18_400_000 };
      case STRATA_METHODS.DAEMON_STATUS: {
        const list = Object.values(this.dataset.containers);
        return {
          pid: 1420,
          uid: 0,
          uptimeMs: 18_400_000,
          defaultContainer: list.find((c) => c.snapshot.isDefault)?.snapshot.id ?? 'main',
          totalContainers: list.length,
          runningContainers: list.filter((c) => c.snapshot.phase === 'running').length,
          activeTasks: 0,
          sideveilConnected: true,
        };
      }
      case STRATA_METHODS.DAEMON_CAPABILITIES:
        return this.dataset.capabilities;
      case STRATA_METHODS.DAEMON_LOGS:
        return { logPath: '/data/adb/strata/run/logs/stratad.log', lines: this.dataset.daemonLogs };
      case STRATA_METHODS.DAEMON_RELOAD_CONFIG:
        return {
          version: 1,
          defaultContainer: 'main',
          web: {
            enabled: this.dataset.webStatus.enabled,
            lanEnabled: this.dataset.webStatus.lanEnabled,
            port: this.dataset.webStatus.port,
          },
          containers: Object.fromEntries(
            Object.entries(this.dataset.containers).map(([k, v]) => [k, v.config]),
          ),
        };
      case STRATA_METHODS.WEB_STATUS:
        return this.dataset.webStatus;
      case STRATA_METHODS.WEB_CONFIGURE: {
        if (typeof p.enabled === 'boolean') this.dataset.webStatus.enabled = p.enabled;
        if (typeof p.lanEnabled === 'boolean') {
          this.dataset.webStatus.lanEnabled = p.lanEnabled;
          this.dataset.webStatus.listenAddr = p.lanEnabled ? '0.0.0.0' : '127.0.0.1';
        }
        if (typeof p.port === 'number') this.dataset.webStatus.port = p.port;
        return this.dataset.webStatus;
      }
      case STRATA_METHODS.WEB_ROTATE_TOKEN:
        return {
          token: `mock_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`,
          tokenPath: this.dataset.webStatus.tokenPath,
          sessionsRevoked: 1,
        };
      case STRATA_METHODS.DISTRO_CATALOG:
        return {
          mirror: (p.mirror as 'tuna' | 'official' | 'custom') ?? 'tuna',
          baseUrl: 'https://mirrors.tuna.tsinghua.edu.cn/lxc-images',
          cached: false,
          fetchedAtMs: Date.now(),
          entries: MOCK_DISTRO_CATALOG,
        };
      case STRATA_METHODS.CONTAINER_LIST:
        return Object.values(this.dataset.containers).map((c) => c.snapshot);
      case STRATA_METHODS.CONTAINER_STATUS:
        return requireMockContainer(this.dataset, p.container as string | undefined).snapshot;
      case STRATA_METHODS.CONTAINER_START:
      case STRATA_METHODS.CONTAINER_RESTART:
        return setMockContainerPhase(ctx, p.container as string | undefined, 'running');
      case STRATA_METHODS.CONTAINER_STOP:
        setMockContainerPhase(ctx, p.container as string | undefined, 'stopped');
        return {
          drainedPids: 6,
          unmountedPaths: 5,
          detachedLoops: 1,
          residualMounts: 0,
          residualLoops: 0,
        };
      case STRATA_METHODS.CONTAINER_COMPACT: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        const before = rec.snapshot.disk.imageAllocatedBytes;
        const trimmed = Math.min(before, 280 * 1024 * 1024);
        rec.snapshot.disk.imageAllocatedBytes = Math.max(0, before - trimmed);
        return {
          container: rec.snapshot.id,
          beforeBytes: before,
          afterBytes: rec.snapshot.disk.imageAllocatedBytes,
          trimmedBytes: trimmed,
        };
      }
      case STRATA_METHODS.CONTAINER_FSCK: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        return { container: rec.snapshot.id, exitCode: 0, output: 'e2fsck 1.47.1: clean' };
      }
      case STRATA_METHODS.CONTAINER_SYNC_DNS:
        return {
          container: requireMockContainer(this.dataset, p.container as string | undefined).snapshot
            .id,
          synced: true,
        };
      case STRATA_METHODS.CONTAINER_LOGS: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        return {
          container: rec.snapshot.id,
          logPath: `/data/adb/strata/run/logs/${rec.snapshot.id}.log`,
          lines: rec.logs,
        };
      }
      case STRATA_METHODS.CONTAINER_CONFIG_GET: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        return { container: rec.snapshot.id, config: rec.config };
      }
      case STRATA_METHODS.CONTAINER_CONFIG_PATCH:
        return patchMockContainerConfig(this.dataset, p);
      case STRATA_METHODS.CONTAINER_SSH_KEY_LIST: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        return { container: rec.snapshot.id, keys: rec.sshKeys, added: 0 };
      }
      case STRATA_METHODS.CONTAINER_SSH_KEY_ADD: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        const key = String(p.publicKey ?? '').trim();
        if (key && !rec.sshKeys.includes(key)) rec.sshKeys.push(key);
        return { container: rec.snapshot.id, keys: rec.sshKeys, added: 1 };
      }
      case STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE: {
        const rec = requireMockContainer(this.dataset, p.id as string | undefined);
        const needle = String(p.fingerprintOrKey ?? '');
        const before = rec.sshKeys.length;
        rec.sshKeys = rec.sshKeys.filter((k) => k !== needle);
        return {
          container: rec.snapshot.id,
          removed: before - rec.sshKeys.length,
          keys: rec.sshKeys,
        };
      }
      case STRATA_METHODS.CONTAINER_RESIZE: {
        const rec = requireMockContainer(this.dataset, p.id as string | undefined);
        const prev = rec.snapshot.disk.imageVirtualBytes;
        const next = Number(p.sizeMb) * 1024 * 1024;
        rec.snapshot.disk.imageVirtualBytes = next;
        return {
          container: rec.snapshot.id,
          previousBytes: prev,
          newBytes: next,
          restarted: rec.snapshot.phase === 'running',
        };
      }
      case STRATA_METHODS.CONTAINER_CLONE:
        return cloneMockContainer(
          ctx,
          String(p.sourceId),
          String(p.targetId),
          p.sshPort as number | undefined,
          p.hostname as string | undefined,
        );
      case STRATA_METHODS.CONTAINER_DELETE:
        return deleteMockContainer(this.dataset, String(p.id), Boolean(p.deleteImage));
      case STRATA_METHODS.CONTAINER_CREATE:
        return startMockCreateJob(ctx, p);
      case STRATA_METHODS.CONTAINER_JOB_STATUS: {
        const job = this.jobs.get(String(p.jobId));
        if (!job) throw new StrataRpcError(-32027, '任务不存在');
        return job;
      }
      case STRATA_METHODS.CONTAINER_JOB_CANCEL: {
        const job = this.jobs.get(String(p.jobId));
        if (job && job.state === 'running') {
          job.state = 'cancelled';
          this.emit(STRATA_EVENTS.CONTAINER_JOB_PROGRESS, { ...job });
          return { jobId: job.jobId, cancelled: true };
        }
        return { jobId: String(p.jobId), cancelled: false };
      }
      case STRATA_METHODS.SERVICE_LIST:
        return requireMockContainer(this.dataset, p.container as string | undefined).services;
      case STRATA_METHODS.SERVICE_UPSERT:
        return upsertMockService(
          ctx,
          p.container as string | undefined,
          p.spec as ContainerConfig['services'][number],
        );
      case STRATA_METHODS.SERVICE_DELETE: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        const sid = String(p.serviceId);
        rec.config.services = rec.config.services.filter((s) => s.id !== sid);
        const before = rec.services.length;
        rec.services = rec.services.filter((s) => s.id !== sid);
        delete rec.serviceLogs[sid];
        return {
          container: rec.snapshot.id,
          serviceId: sid,
          deleted: before !== rec.services.length,
        };
      }
      case STRATA_METHODS.SERVICE_START:
        return setMockServiceStatus(
          ctx,
          p.container as string | undefined,
          String(p.serviceId),
          'running',
        );
      case STRATA_METHODS.SERVICE_STOP:
        return setMockServiceStatus(
          ctx,
          p.container as string | undefined,
          String(p.serviceId),
          'stopped',
        );
      case STRATA_METHODS.SERVICE_RESTART:
        return setMockServiceStatus(
          ctx,
          p.container as string | undefined,
          String(p.serviceId),
          'running',
          true,
        );
      case STRATA_METHODS.SERVICE_LOGS: {
        const rec = requireMockContainer(this.dataset, p.container as string | undefined);
        const sid = String(p.serviceId);
        return {
          container: rec.snapshot.id,
          serviceId: sid,
          lines: rec.serviceLogs[sid] ?? ['[service] 暂无日志输出'],
        };
      }
      case STRATA_METHODS.CONTAINER_BACKUP:
        return {
          container: String(p.container ?? 'main'),
          outputPath: String(p.outputPath),
          bytesWritten: 1_200_000_000,
          restarted: false,
        };
    }
  }

  subscribe(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  openPty(
    containerId: string,
    _cols: number,
    _rows: number,
    handlers: PtyConnectionHandlers,
    resumeSessionId?: string,
  ): PtySessionHandle {
    const hostname = this.dataset.containers[containerId]?.snapshot.hostname ?? containerId;
    return createMockPtySession(containerId, hostname, handlers, resumeSessionId);
  }

  async killPty(_sessionId: string): Promise<void> {}

  async uploadImage(options: ImportUploadOptions): Promise<ContainerJobProgressPayload> {
    const total = Math.max(1, options.file.size);
    for (const ratio of [0.25, 0.6, 1]) {
      if (options.signal?.aborted) throw new Error('用户取消上传');
      options.onProgress?.(Math.round(total * ratio), total);
      await new Promise((r) => setTimeout(r, 120));
    }
    registerNewMockContainer(
      this.ctx(),
      options.containerId,
      options.sizeMb,
      options.autoStart,
      options.sshPort,
    );
    const now = Date.now();
    return {
      jobId: `import-${now.toString(36)}`,
      container: options.containerId,
      kind: 'import',
      state: 'completed',
      stage: 'done',
      bytesDone: total,
      bytesTotal: total,
      percent: 100,
      message: '本地镜像导入完成',
      startedAtMs: now,
      updatedAtMs: now,
    };
  }

  async downloadExport(_containerId: string, _format: 'tar.gz' | 'img.gz'): Promise<void> {}

  dispose(): void {
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    this.listeners.clear();
  }
}
