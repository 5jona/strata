import {
  type BindMount,
  type ContainerConfig,
  type ContainerStatusSnapshot,
  type HudStateResult,
  STRATA_PATHS,
  STRATA_PROTOCOL,
  type TaskOutputPayload,
  type TaskSummary,
} from '../../types.ts';

/** In-memory task summary and captured log chunks for the mock preview transport. */
export interface MockTaskRecord {
  summary: TaskSummary;
  entries: TaskOutputPayload[];
}

/** Mutable state container backing the standalone browser preview and test mock transport. */
export interface MockStrataState {
  defaultId: string;
  container: ContainerStatusSnapshot;
  config: ContainerConfig;
  sshKeys: string[];
  dropbearLogs: string[];
  hud: HudStateResult;
  tasks: MockTaskRecord[];
  nextTaskSeq: number;
}

/** Type guard verifying that an unknown value is a plain key-value object. */
export function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Safely coerces raw RPC params into a plain record without unchecked type assertions. */
export function toParamsRecord(raw: unknown): Record<string, unknown> {
  return isPlainRecord(raw) ? raw : {};
}

/** Builds the initial synthetic container, config, SSH key, and HUD state for preview mode. */
export function createInitialMockState(): MockStrataState {
  const defaultId = STRATA_PROTOCOL.DEFAULT_CONTAINER_ID;
  const imagePath = `${STRATA_PATHS.DEFAULT_CONTAINERS_ROOT}/${defaultId}/rootfs.img`;
  const mountDir = `${STRATA_PATHS.DEFAULT_CONTAINERS_ROOT}/${defaultId}/mnt`;
  const hostname = `strata-${defaultId}`;
  return {
    defaultId,
    container: {
      id: defaultId,
      isDefault: true,
      enabled: true,
      autoStart: true,
      phase: 'running',
      nsPid: 12549,
      cgroupPath: `${STRATA_PATHS.CGROUP_V2_ROOT}/${STRATA_PATHS.CGROUP_V2_NAMESPACE}/${defaultId}`,
      mounted: true,
      mountDir,
      imagePath,
      loopDevices: ['/dev/block/loop9'],
      hostname,
      sshEnabled: true,
      sshListenAddr: STRATA_PROTOCOL.DEFAULT_SSH_LISTEN_ADDR,
      sshPort: STRATA_PROTOCOL.DEFAULT_SSH_PORT,
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
    },
    config: {
      enabled: true,
      autoStart: true,
      imagePath,
      mountDir,
      hostname,
      ssh: {
        enabled: true,
        listenAddr: STRATA_PROTOCOL.DEFAULT_SSH_LISTEN_ADDR,
        port: STRATA_PROTOCOL.DEFAULT_SSH_PORT,
        keepaliveSecs: STRATA_PROTOCOL.DEFAULT_SSH_KEEPALIVE_SECS,
      },
      oomScoreAdj: STRATA_PROTOCOL.DEFAULT_OOM_SCORE_ADJ,
      hostBridge: true,
      hostRuntime: false,
      bindDataLocalTmp: true,
      extraCharDevs: [STRATA_PROTOCOL.DEFAULT_EXTRA_CHAR_DEV],
      extraMounts: [],
      syncAndroidDns: true,
      dnsServers: ['223.5.5.5', '1.1.1.1', '8.8.8.8'],
      fsckOnMount: true,
      initCommand: [],
    },
    sshKeys: ['ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockPreviewKeyStrata dev@workstation'],
    dropbearLogs: [
      '[12549] Not backgrounding',
      `[12549] Listening on 0.0.0.0:${STRATA_PROTOCOL.DEFAULT_SSH_PORT} (key-only)`,
      '[12610] Pubkey auth succeeded for root',
    ],
    hud: { enabled: false, clickThrough: false, isDark: true, sideveilConnected: true },
    tasks: [],
    nextTaskSeq: 1,
  };
}

/** Deep-clones a container status snapshot so mock callers cannot mutate internal state. */
export function cloneSnapshot(c: ContainerStatusSnapshot): ContainerStatusSnapshot {
  return { ...c, loopDevices: [...c.loopDevices], disk: { ...c.disk } };
}

/** Deep-clones a container configuration object for mock RPC responses. */
export function cloneConfig(cfg: ContainerConfig): ContainerConfig {
  return {
    ...cfg,
    ssh: { ...cfg.ssh },
    extraCharDevs: [...cfg.extraCharDevs],
    extraMounts: cfg.extraMounts.map((m) => ({ ...m })),
    dnsServers: [...cfg.dnsServers],
    initCommand: [...cfg.initCommand],
  };
}

function parseBindMount(entry: unknown): BindMount | null {
  if (!isPlainRecord(entry)) return null;
  if (typeof entry.hostPath !== 'string' || typeof entry.containerPath !== 'string') return null;
  return {
    hostPath: entry.hostPath,
    containerPath: entry.containerPath,
    readOnly: Boolean(entry.readOnly),
  };
}

/** Applies a partial configuration patch onto both the mock config and live container snapshot. */
export function applyMockConfigPatch(
  config: ContainerConfig,
  container: ContainerStatusSnapshot,
  params: Record<string, unknown>,
): void {
  if (typeof params.enabled === 'boolean') config.enabled = container.enabled = params.enabled;
  if (typeof params.autoStart === 'boolean')
    config.autoStart = container.autoStart = params.autoStart;
  if (typeof params.imagePath === 'string')
    config.imagePath = container.imagePath = params.imagePath;
  if (typeof params.mountDir === 'string') config.mountDir = container.mountDir = params.mountDir;
  if (typeof params.hostname === 'string') config.hostname = container.hostname = params.hostname;
  if (typeof params.sshPort === 'number') config.ssh.port = container.sshPort = params.sshPort;
  if (typeof params.hostBridge === 'boolean')
    config.hostBridge = container.hostBridge = params.hostBridge;
  if (typeof params.hostRuntime === 'boolean')
    config.hostRuntime = container.hostRuntime = params.hostRuntime;
  if (Array.isArray(params.extraCharDevs)) config.extraCharDevs = params.extraCharDevs.map(String);
  if (Array.isArray(params.extraMounts)) {
    const parsedMounts: BindMount[] = [];
    for (const item of params.extraMounts) {
      const mount = parseBindMount(item);
      if (mount) parsedMounts.push(mount);
    }
    config.extraMounts = parsedMounts;
  }
}
