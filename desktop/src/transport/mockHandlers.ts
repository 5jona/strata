import type {
  ContainerConfig,
  ContainerJobProgressPayload,
  ContainerServiceRuntime,
  ContainerStatusSnapshot,
} from '../types/protocol.ts';
import {
  STRATA_EVENTS,
  type StrataDesktopEventMap,
  type StrataDesktopEventName,
  StrataRpcError,
} from '../types/rpc.ts';
import type { MockContainerRecord, MockDeviceDataset } from './mockData.ts';

export interface MockRuntimeContext {
  dataset: MockDeviceDataset;
  jobs: Map<string, ContainerJobProgressPayload>;
  emit: <E extends StrataDesktopEventName>(event: E, payload: StrataDesktopEventMap[E]) => void;
  schedule: (delayMs: number, fn: () => void) => void;
}

export function requireMockContainer(dataset: MockDeviceDataset, id?: string): MockContainerRecord {
  const targetId = id ?? Object.keys(dataset.containers)[0] ?? 'main';
  const rec = dataset.containers[targetId];
  if (!rec) throw new StrataRpcError(-32021, `容器不存在: ${targetId}`);
  return rec;
}

export function setMockContainerPhase(
  ctx: MockRuntimeContext,
  id: string | undefined,
  next: 'running' | 'stopped',
): ContainerStatusSnapshot {
  const rec = requireMockContainer(ctx.dataset, id);
  const prev = rec.snapshot.phase;
  rec.snapshot.phase = next;
  rec.snapshot.mounted = next === 'running';
  rec.snapshot.nsPid = next === 'running' ? 19100 : undefined;
  rec.snapshot.uptimeMs = next === 'running' ? 1000 : 0;
  for (const svc of rec.services) {
    if (next === 'stopped') {
      svc.status = 'stopped';
      svc.pid = undefined;
    } else if (svc.autoStart) {
      svc.status = 'running';
      svc.pid = 19200;
    }
  }
  ctx.emit(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
    container: rec.snapshot.id,
    previous: prev,
    current: next,
    snapshot: structuredClone(rec.snapshot),
  });
  return rec.snapshot;
}

export function registerNewMockContainer(
  ctx: MockRuntimeContext,
  id: string,
  sizeMb: number,
  autoStart: boolean,
  sshPort?: number,
): void {
  const base = structuredClone(requireMockContainer(ctx.dataset));
  base.snapshot.id = id;
  base.snapshot.isDefault = false;
  base.snapshot.autoStart = autoStart;
  base.snapshot.phase = autoStart ? 'running' : 'stopped';
  base.snapshot.mounted = autoStart;
  base.snapshot.hostname = id;
  base.snapshot.sshPort = sshPort ?? 2224;
  base.snapshot.disk = {
    imageExists: true,
    imageAllocatedBytes: 420 * 1024 * 1024,
    imageVirtualBytes: sizeMb * 1024 * 1024,
  };
  base.config.hostname = id;
  base.config.autoStart = autoStart;
  base.config.ssh.port = base.snapshot.sshPort;
  base.config.services = [];
  base.services = [];
  base.serviceLogs = {};
  ctx.dataset.containers[id] = base;
  ctx.emit(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
    container: id,
    previous: 'stopped',
    current: base.snapshot.phase,
    snapshot: structuredClone(base.snapshot),
  });
}

export function startMockCreateJob(ctx: MockRuntimeContext, p: Record<string, unknown>) {
  const id = String(p.id ?? '');
  if (!id || ctx.dataset.containers[id]) {
    throw new StrataRpcError(-32026, `容器 ID 无效或已存在: ${id}`);
  }
  const jobId = `job-${Date.now().toString(36)}`;
  const now = Date.now();
  const job: ContainerJobProgressPayload = {
    jobId,
    container: id,
    kind: 'create',
    state: 'running',
    stage: 'downloading',
    bytesDone: 0,
    bytesTotal: 120 * 1024 * 1024,
    percent: 8,
    message: '正在从镜像站拉取 ARM64 RootFS…',
    startedAtMs: now,
    updatedAtMs: now,
  };
  ctx.jobs.set(jobId, job);
  ctx.emit(STRATA_EVENTS.CONTAINER_JOB_PROGRESS, { ...job });

  const steps = [
    { delay: 450, stage: 'extracting', percent: 55, message: '正在创建 ext4 稀疏镜像并解压…' },
    { delay: 950, stage: 'provisioning', percent: 85, message: '配置 Dropbear SSH 与 DNS…' },
    { delay: 1400, stage: 'done', percent: 100, message: '容器初始化完成' },
  ];
  for (const step of steps) {
    ctx.schedule(step.delay, () => {
      if (job.state === 'cancelled') return;
      job.stage = step.stage;
      job.percent = step.percent;
      job.bytesDone = Math.round((job.bytesTotal ?? 100) * (step.percent / 100));
      job.message = step.message;
      job.updatedAtMs = Date.now();
      if (step.percent === 100) {
        job.state = 'completed';
        registerNewMockContainer(
          ctx,
          id,
          Number(p.sizeMb ?? 16384),
          Boolean(p.autoStart),
          p.sshPort as number | undefined,
        );
      }
      ctx.emit(STRATA_EVENTS.CONTAINER_JOB_PROGRESS, { ...job });
    });
  }
  return { jobId, container: id, kind: 'create' as const };
}

export function cloneMockContainer(
  ctx: MockRuntimeContext,
  sourceId: string,
  targetId: string,
  sshPort?: number,
  hostname?: string,
) {
  const src = requireMockContainer(ctx.dataset, sourceId);
  if (ctx.dataset.containers[targetId]) {
    throw new StrataRpcError(-32026, `容器已存在: ${targetId}`);
  }
  const cloned = structuredClone(src);
  cloned.snapshot.id = targetId;
  cloned.snapshot.isDefault = false;
  cloned.snapshot.phase = 'stopped';
  cloned.snapshot.mounted = false;
  cloned.snapshot.hostname = hostname ?? targetId;
  cloned.snapshot.sshPort = sshPort ?? src.snapshot.sshPort + 1;
  cloned.snapshot.imagePath = `/data/local/strata/containers/${targetId}/rootfs.img`;
  cloned.config.hostname = cloned.snapshot.hostname;
  cloned.config.ssh.port = cloned.snapshot.sshPort;
  cloned.config.imagePath = cloned.snapshot.imagePath;
  ctx.dataset.containers[targetId] = cloned;
  return {
    sourceId,
    targetId,
    imagePath: cloned.snapshot.imagePath,
    copiedBytes: cloned.snapshot.disk.imageAllocatedBytes,
    virtualBytes: cloned.snapshot.disk.imageVirtualBytes,
  };
}

export function upsertMockService(
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
    runtime.name = spec.name;
    runtime.autoStart = spec.autoStart;
  }
  return runtime;
}

export function setMockServiceStatus(
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
  const line = `${new Date().toISOString()} [${serviceId}] transitioned ${prev} -> ${next}`;
  rec.serviceLogs[serviceId] = [...(rec.serviceLogs[serviceId] ?? []), line];
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

export function patchMockContainerConfig(dataset: MockDeviceDataset, p: Record<string, unknown>) {
  const rec = requireMockContainer(dataset, p.container as string | undefined);
  if (typeof p.hostname === 'string') {
    rec.config.hostname = p.hostname;
    rec.snapshot.hostname = p.hostname;
  }
  if (typeof p.sshPort === 'number') {
    rec.config.ssh.port = p.sshPort;
    rec.snapshot.sshPort = p.sshPort;
  }
  if (typeof p.autoStart === 'boolean') {
    rec.config.autoStart = p.autoStart;
    rec.snapshot.autoStart = p.autoStart;
  }
  if (typeof p.hostBridge === 'boolean') {
    rec.config.hostBridge = p.hostBridge;
    rec.snapshot.hostBridge = p.hostBridge;
  }
  if (typeof p.hostRuntime === 'boolean') {
    rec.config.hostRuntime = p.hostRuntime;
    rec.snapshot.hostRuntime = p.hostRuntime;
  }
  if (typeof p.noatime === 'boolean') rec.config.noatime = p.noatime;
  return { container: rec.snapshot.id, config: rec.config };
}

export function deleteMockContainer(dataset: MockDeviceDataset, id: string, deleteImage: boolean) {
  if (!dataset.containers[id]) throw new StrataRpcError(-32021, `容器不存在: ${id}`);
  delete dataset.containers[id];
  const remaining = Object.keys(dataset.containers);
  return {
    container: id,
    imageDeleted: deleteImage,
    cleanup: {
      drainedPids: 0,
      unmountedPaths: 0,
      detachedLoops: 0,
      residualMounts: 0,
      residualLoops: 0,
    },
    ...(remaining[0] ? { newDefaultContainer: remaining[0] } : {}),
  };
}
