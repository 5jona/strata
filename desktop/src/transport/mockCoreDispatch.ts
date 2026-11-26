import {
  type ContainerConfig,
  type ContainerServiceRuntime,
  STRATA_PATHS,
} from '../types/protocol.ts';
import { STRATA_EVENTS, STRATA_METHODS, StrataRpcError } from '../types/rpc.ts';
import {
  cloneMockContainer,
  deleteMockContainer,
  type MockRuntimeContext,
  patchMockContainerConfig,
  requireMockContainer,
  setMockContainerPhase,
  setMockDefaultContainer,
  startMockCreateJob,
} from './mockHandlers.ts';

function upsertMockService(
  ctx: MockRuntimeContext,
  containerId: string | undefined,
  spec: ContainerConfig['services'][number],
): ContainerServiceRuntime {
  const rec = requireMockContainer(ctx.dataset, containerId);
  const idx = rec.config.services.findIndex((s) => s.id === spec.id);
  if (idx >= 0) rec.config.services[idx] = spec;
  else rec.config.services.push(spec);
  let runtime = rec.services.find((s) => s.id === spec.id);
  if (!runtime) {
    runtime = {
      id: spec.id,
      name: spec.name,
      status: 'stopped',
      uptimeMs: 0,
      restartCount: 0,
      autoStart: spec.autoStart,
    };
    rec.services.push(runtime);
    rec.serviceLogs[spec.id] = [`[service:${spec.id}] 已注册服务，等待启动`];
  } else {
    Object.assign(runtime, { name: spec.name, autoStart: spec.autoStart });
  }
  return runtime;
}

function setMockServiceStatus(
  ctx: MockRuntimeContext,
  containerId: string | undefined,
  serviceId: string,
  next: 'running' | 'stopped',
  incrementRestart = false,
): ContainerServiceRuntime {
  const rec = requireMockContainer(ctx.dataset, containerId);
  const svc = rec.services.find((s) => s.id === serviceId);
  if (!svc) throw new StrataRpcError(-32028, `服务不存在: ${serviceId}`);
  const prev = svc.status;
  svc.status = next;
  svc.pid = next === 'running' ? 20_000 + Math.floor(Math.random() * 900) : undefined;
  svc.uptimeMs = next === 'running' ? 1000 : 0;
  if (incrementRestart) svc.restartCount += 1;
  rec.serviceLogs[serviceId] = [
    ...(rec.serviceLogs[serviceId] ?? []),
    `${new Date().toISOString()} [${serviceId}] transitioned ${prev} -> ${next}`,
  ];
  ctx.emit(STRATA_EVENTS.SERVICE_STATE_CHANGED, {
    container: rec.snapshot.id,
    serviceId,
    prevStatus: prev,
    status: next,
    pid: svc.pid,
    restartCount: svc.restartCount,
  });
  return svc;
}

export function dispatchMockContainerAndServiceRpc(
  ctx: MockRuntimeContext,
  method: string,
  p: Record<string, unknown>,
): unknown {
  const { dataset, jobs, emit } = ctx;
  switch (method) {
    case STRATA_METHODS.CONTAINER_LIST:
      return Object.values(dataset.containers).map((c) => c.snapshot);
    case STRATA_METHODS.CONTAINER_STATUS:
      return requireMockContainer(dataset, p.container as string | undefined).snapshot;
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
      const rec = requireMockContainer(dataset, p.container as string | undefined);
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
    case STRATA_METHODS.CONTAINER_FSCK:
      return {
        container: requireMockContainer(dataset, p.container as string | undefined).snapshot.id,
        exitCode: 0,
        output: 'e2fsck 1.47.1: clean',
      };
    case STRATA_METHODS.CONTAINER_SYNC_DNS:
      return {
        container: requireMockContainer(dataset, p.container as string | undefined).snapshot.id,
        synced: true,
      };
    case STRATA_METHODS.CONTAINER_LOGS: {
      const rec = requireMockContainer(dataset, p.container as string | undefined);
      return {
        container: rec.snapshot.id,
        logPath: `${STRATA_PATHS.DEFAULT_LOGS_DIR}/${rec.snapshot.id}.log`,
        lines: rec.logs,
      };
    }
    case STRATA_METHODS.CONTAINER_CONFIG_GET: {
      const rec = requireMockContainer(dataset, p.container as string | undefined);
      return { container: rec.snapshot.id, config: rec.config };
    }
    case STRATA_METHODS.CONTAINER_CONFIG_PATCH:
      return patchMockContainerConfig(dataset, p);
    case STRATA_METHODS.CONTAINER_SSH_KEY_LIST: {
      const rec = requireMockContainer(dataset, p.container as string | undefined);
      return { container: rec.snapshot.id, keys: rec.sshKeys, added: 0 };
    }
    case STRATA_METHODS.CONTAINER_SSH_KEY_ADD: {
      const rec = requireMockContainer(dataset, p.container as string | undefined);
      const key = String(p.publicKey ?? '').trim();
      if (key && !rec.sshKeys.includes(key)) rec.sshKeys.push(key);
      return { container: rec.snapshot.id, keys: rec.sshKeys, added: 1 };
    }
    case STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE: {
      const rec = requireMockContainer(dataset, p.id as string | undefined);
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
      const rec = requireMockContainer(dataset, p.id as string | undefined);
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
      return deleteMockContainer(dataset, String(p.id), Boolean(p.deleteImage));
    case STRATA_METHODS.CONTAINER_SET_DEFAULT:
      return setMockDefaultContainer(ctx, String(p.id));
    case STRATA_METHODS.CONTAINER_CREATE:
      return startMockCreateJob(ctx, p);
    case STRATA_METHODS.CONTAINER_JOB_STATUS: {
      const job = jobs.get(String(p.jobId));
      if (!job) throw new StrataRpcError(-32027, '任务不存在');
      return job;
    }
    case STRATA_METHODS.CONTAINER_JOB_CANCEL: {
      const job = jobs.get(String(p.jobId));
      if (job && job.state === 'running') {
        job.state = 'cancelled';
        emit(STRATA_EVENTS.CONTAINER_JOB_PROGRESS, { ...job });
        return { jobId: job.jobId, cancelled: true };
      }
      return { jobId: String(p.jobId), cancelled: false };
    }
    case STRATA_METHODS.CONTAINER_BACKUP:
    case STRATA_METHODS.CONTAINER_BACKUP_START:
      return {
        container: String(p.container ?? 'main'),
        outputPath: String(p.outputPath),
        bytesWritten: 1_200_000_000,
        restarted: false,
      };
    case STRATA_METHODS.SERVICE_LIST:
      return requireMockContainer(dataset, p.container as string | undefined).services;
    case STRATA_METHODS.SERVICE_UPSERT:
      return upsertMockService(
        ctx,
        p.container as string | undefined,
        p.spec as ContainerConfig['services'][number],
      );
    case STRATA_METHODS.SERVICE_DELETE: {
      const rec = requireMockContainer(dataset, p.container as string | undefined);
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
      const rec = requireMockContainer(dataset, p.container as string | undefined);
      const sid = String(p.serviceId);
      return {
        container: rec.snapshot.id,
        serviceId: sid,
        lines: rec.serviceLogs[sid] ?? ['[service] 暂无日志输出'],
      };
    }
    default:
      return undefined;
  }
}
