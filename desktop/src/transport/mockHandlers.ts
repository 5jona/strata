import type {
  ContainerConfig,
  ContainerJobProgressPayload,
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

export interface RegisterMockContainerOpts {
  id: string;
  sizeMb: number;
  hostname?: string;
  enableSsh: boolean;
  sshPort?: number;
  sshPublicKeys?: string[];
  autoStart: boolean;
  setDefault: boolean;
  hostBridge: boolean;
  extraCharDevs?: string[];
  l5DiagnosticMounts: boolean;
  syncAndroidDns: boolean;
  noatime: boolean;
}

export function registerNewMockContainer(
  ctx: MockRuntimeContext,
  opts: RegisterMockContainerOpts,
): void {
  const base = structuredClone(requireMockContainer(ctx.dataset));
  if (opts.setDefault) {
    for (const item of Object.values(ctx.dataset.containers)) {
      item.snapshot.isDefault = false;
    }
  }
  const effectivePort = opts.enableSsh ? (opts.sshPort ?? 2224) : 0;
  const hostname = opts.hostname?.trim() || opts.id;
  Object.assign(base.snapshot, {
    id: opts.id,
    isDefault: opts.setDefault,
    autoStart: opts.autoStart,
    phase: opts.autoStart ? 'running' : 'stopped',
    mounted: opts.autoStart,
    hostname,
    sshEnabled: opts.enableSsh,
    sshPort: effectivePort,
    hostBridge: opts.hostBridge,
    disk: {
      imageExists: true,
      imageAllocatedBytes: 420 * 1024 * 1024,
      imageVirtualBytes: opts.sizeMb * 1024 * 1024,
    },
  });
  Object.assign(base.config, {
    hostname,
    autoStart: opts.autoStart,
    hostBridge: opts.hostBridge,
    syncAndroidDns: opts.syncAndroidDns,
    noatime: opts.noatime,
    extraCharDevs: opts.extraCharDevs ?? (opts.hostBridge ? ['uinput'] : []),
    services: [],
    extraMounts: opts.l5DiagnosticMounts
      ? ['/sys/fs/pstore', '/sys/kernel/tracing', '/sys/fs/bpf'].map((p) => ({
          hostPath: p,
          containerPath: p,
          readOnly: true,
        }))
      : [],
  });
  base.config.ssh.enabled = opts.enableSsh;
  base.config.ssh.port = effectivePort;
  base.services = [];
  base.serviceLogs = {};
  base.sshKeys = opts.enableSsh ? [...(opts.sshPublicKeys ?? [])] : [];
  ctx.dataset.containers[opts.id] = base;
  ctx.emit(STRATA_EVENTS.CONTAINER_STATE_CHANGED, {
    container: opts.id,
    previous: 'stopped',
    current: base.snapshot.phase,
    snapshot: structuredClone(base.snapshot),
  });
}

export function setMockDefaultContainer(ctx: MockRuntimeContext, id: string) {
  const target = requireMockContainer(ctx.dataset, id);
  const prev = Object.values(ctx.dataset.containers).find((c) => c.snapshot.isDefault)?.snapshot.id;
  for (const c of Object.values(ctx.dataset.containers)) {
    c.snapshot.isDefault = c.snapshot.id === target.snapshot.id;
  }
  return {
    defaultContainer: target.snapshot.id,
    ...(prev ? { previousDefaultContainer: prev } : {}),
  };
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
    message: p.sourceUrl ? '正在从直链下载镜像…' : '正在从镜像站拉取 ARM64 RootFS…',
    startedAtMs: now,
    updatedAtMs: now,
  };
  ctx.jobs.set(jobId, job);
  ctx.emit(STRATA_EVENTS.CONTAINER_JOB_PROGRESS, { ...job });

  const str = (v: unknown) => (typeof v === 'string' ? v : undefined);
  const steps = [
    { delay: 450, stage: 'extracting', percent: 55, message: '正在创建 ext4 稀疏镜像并解压…' },
    { delay: 950, stage: 'provisioning', percent: 85, message: '初始化容器运行环境…' },
    { delay: 1400, stage: 'completed', percent: 100, message: '容器初始化完成' },
  ];
  for (const step of steps) {
    ctx.schedule(step.delay, () => {
      if (job.state === 'cancelled') return;
      Object.assign(job, {
        stage: step.stage,
        percent: step.percent,
        bytesDone: Math.round((job.bytesTotal ?? 100) * (step.percent / 100)),
        message: step.message,
        updatedAtMs: Date.now(),
      });
      if (step.percent === 100) {
        job.state = 'completed';
        registerNewMockContainer(ctx, {
          id,
          sizeMb: Number(p.sizeMb ?? 16384),
          ...(str(p.hostname) ? { hostname: str(p.hostname) } : {}),
          enableSsh: Boolean(p.enableSsh),
          ...(typeof p.sshPort === 'number' ? { sshPort: p.sshPort } : {}),
          sshPublicKeys: Array.isArray(p.sshPublicKeys) ? p.sshPublicKeys.map(String) : [],
          autoStart: Boolean(p.autoStart),
          setDefault: Boolean(p.setDefault),
          hostBridge: p.hostBridge !== false,
          ...(Array.isArray(p.extraCharDevs) ? { extraCharDevs: p.extraCharDevs.map(String) } : {}),
          l5DiagnosticMounts: Boolean(p.l5DiagnosticMounts),
          syncAndroidDns: p.syncAndroidDns !== false,
          noatime: p.noatime !== false,
        });
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
  if (ctx.dataset.containers[targetId]) throw new StrataRpcError(-32026, `容器已存在: ${targetId}`);
  const cloned = structuredClone(src);
  const nextHostname = hostname ?? targetId;
  const nextPort = sshPort ?? src.snapshot.sshPort + 1;
  const imagePath = `/data/local/strata/containers/${targetId}/rootfs.img`;
  Object.assign(cloned.snapshot, {
    id: targetId,
    isDefault: false,
    phase: 'stopped',
    mounted: false,
    hostname: nextHostname,
    sshPort: nextPort,
    imagePath,
  });
  Object.assign(cloned.config, { hostname: nextHostname, imagePath });
  cloned.config.ssh.port = nextPort;
  ctx.dataset.containers[targetId] = cloned;
  return {
    sourceId,
    targetId,
    imagePath,
    copiedBytes: cloned.snapshot.disk.imageAllocatedBytes,
    virtualBytes: cloned.snapshot.disk.imageVirtualBytes,
  };
}

export function patchMockContainerConfig(dataset: MockDeviceDataset, p: Record<string, unknown>) {
  const rec = requireMockContainer(dataset, p.container as string | undefined);
  for (const k of ['enabled', 'autoStart', 'hostBridge', 'hostRuntime'] as const) {
    if (typeof p[k] === 'boolean') {
      rec.config[k] = p[k];
      rec.snapshot[k] = p[k];
    }
  }
  if (typeof p.hostname === 'string') {
    rec.config.hostname = p.hostname;
    rec.snapshot.hostname = p.hostname;
  }
  if (typeof p.sshPort === 'number') {
    rec.config.ssh.port = p.sshPort;
    rec.snapshot.sshPort = p.sshPort;
  }
  if (typeof p.noatime === 'boolean') rec.config.noatime = p.noatime;
  if (Array.isArray(p.extraCharDevs)) rec.config.extraCharDevs = p.extraCharDevs.map(String);
  if (Array.isArray(p.extraMounts))
    rec.config.extraMounts = p.extraMounts as ContainerConfig['extraMounts'];
  return { container: rec.snapshot.id, config: rec.config };
}

export function deleteMockContainer(dataset: MockDeviceDataset, id: string, deleteImage: boolean) {
  if (!dataset.containers[id]) throw new StrataRpcError(-32021, `容器不存在: ${id}`);
  delete dataset.containers[id];
  const remaining = Object.keys(dataset.containers);
  const cleanup = {
    drainedPids: 0,
    unmountedPaths: 0,
    detachedLoops: 0,
    residualMounts: 0,
    residualLoops: 0,
  };
  return {
    container: id,
    imageDeleted: deleteImage,
    cleanup,
    ...(remaining[0] ? { newDefaultContainer: remaining[0] } : {}),
  };
}
