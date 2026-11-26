import {
  type ContainerServiceRuntime,
  type ContainerServiceSpec,
  STRATA_EVENTS,
  STRATA_METHODS,
  STRATA_PATHS,
} from '../../types.ts';
import type { MockEmitPluginFn } from './daemonHandlers.ts';
import {
  applyMockConfigPatch,
  cloneConfig,
  cloneSnapshot,
  isPlainRecord,
  type MockStrataState,
} from './state.ts';

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

function upsertMockService(state: MockStrataState, rawSpec: unknown): ContainerServiceRuntime {
  if (!isPlainRecord(rawSpec) || typeof rawSpec.id !== 'string') {
    throw new Error('invalid service spec');
  }
  const spec = rawSpec as unknown as ContainerServiceSpec;
  const idx = state.config.services.findIndex((s) => s.id === spec.id);
  if (idx >= 0) {
    state.config.services[idx] = { ...spec };
  } else {
    state.config.services.push({ ...spec });
  }
  const runtime = state.services.find((s) => s.id === spec.id);
  if (runtime) {
    runtime.name = spec.name;
    runtime.autoStart = spec.autoStart;
    return { ...runtime };
  }
  const created: ContainerServiceRuntime = {
    id: spec.id,
    name: spec.name,
    status: 'stopped',
    uptimeMs: 0,
    restartCount: 0,
    autoStart: spec.autoStart,
  };
  state.services.push(created);
  return { ...created };
}

/** Dispatches container lifecycle, maintenance, SSH key, and service RPCs against mock state. */
export function handleContainerMockInvoke(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  method: string,
  params: Record<string, unknown>,
): unknown {
  const { defaultId, container, config, sshKeys, dropbearLogs } = state;

  switch (method) {
    case STRATA_METHODS.SERVICE_LIST:
      return state.services.map((s) => ({ ...s }));
    case STRATA_METHODS.SERVICE_UPSERT:
      return upsertMockService(state, params.spec);
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
        lines: [...(state.serviceLogs[sid] ?? [])],
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
    case STRATA_METHODS.CONTAINER_SET_DEFAULT: {
      const nextDefault = String(params.id ?? defaultId);
      const previousDefaultContainer = state.defaultId;
      state.defaultId = nextDefault;
      container.isDefault = container.id === nextDefault;
      return { defaultContainer: nextDefault, previousDefaultContainer };
    }
    case STRATA_METHODS.CONTAINER_DELETE: {
      const targetId = String(params.id ?? container.id);
      const imageDeleted = Boolean(params.deleteImage);
      return {
        container: targetId,
        imageDeleted,
        cleanup: {
          drainedPids: 0,
          unmountedPaths: 0,
          detachedLoops: 0,
          residualMounts: 0,
          residualLoops: 0,
        },
      };
    }
    case STRATA_METHODS.CONTAINER_CREATE: {
      const targetId = String(params.id ?? defaultId);
      return {
        jobId: `job-create-${targetId}-${Date.now()}`,
        container: targetId,
        kind: 'create',
      };
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
      const defaultPath = `${STRATA_PATHS.DEFAULT_BACKUP_DIR}/strata-${container.id}-${Date.now()}.img.gz`;
      const outputPath =
        typeof params.outputPath === 'string' && params.outputPath.trim()
          ? params.outputPath.trim()
          : defaultPath;
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
    case STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE: {
      const target = String(params.fingerprintOrKey ?? '').trim();
      const idx = sshKeys.findIndex((k) => k === target || k.includes(target));
      let removed = 0;
      if (idx >= 0) {
        sshKeys.splice(idx, 1);
        removed = 1;
      }
      return { container: container.id, removed, keys: [...sshKeys] };
    }
    default:
      return { ok: true };
  }
}
