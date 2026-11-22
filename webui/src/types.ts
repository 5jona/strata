export const STRATA_METHODS = {
  DAEMON_PING: 'daemon.ping',
  DAEMON_STATUS: 'daemon.status',
  DAEMON_RELOAD_CONFIG: 'daemon.reloadConfig',
  CONTAINER_LIST: 'container.list',
  CONTAINER_STATUS: 'container.status',
  CONTAINER_START: 'container.start',
  CONTAINER_STOP: 'container.stop',
  CONTAINER_RESTART: 'container.restart',
  CONTAINER_LOGS: 'container.logs',
  CONTAINER_SYNC_DNS: 'container.syncDns',
  CONTAINER_COMPACT: 'container.compact',
  CONTAINER_FSCK: 'container.fsck',
  CONTAINER_BACKUP_START: 'container.backupStart',
  CONTAINER_CONFIG_GET: 'container.configGet',
  CONTAINER_CONFIG_PATCH: 'container.configPatch',
  CONTAINER_SSH_KEY_LIST: 'container.sshKeyList',
  CONTAINER_SSH_KEY_ADD: 'container.sshKeyAdd',
  TASK_RUN: 'task.run',
  TASK_CANCEL: 'task.cancel',
  TASK_LIST: 'task.list',
  TASK_LOGS: 'task.logs',
  HUD_STATE: 'hud.state',
  HUD_SET: 'hud.set',
} as const;

export const STRATA_EVENTS = {
  CONTAINER_STATE_CHANGED: 'container.stateChanged',
  CONTAINER_MAINTENANCE_DONE: 'container.maintenanceDone',
  TASK_OUTPUT: 'task.output',
  TASK_EXIT: 'task.exit',
  PLUGIN_EVENT_LAGGED: 'plugin.eventLagged',
} as const;

export type ContainerPhase = 'stopped' | 'starting' | 'running' | 'stopping' | 'failed';

export interface ContainerDiskStats {
  imageExists: boolean;
  imageAllocatedBytes: number;
  imageVirtualBytes: number;
  rootfsTotalBytes?: number;
  rootfsUsedBytes?: number;
  rootfsAvailableBytes?: number;
}

export interface ContainerStatusSnapshot {
  id: string;
  isDefault: boolean;
  enabled: boolean;
  autoStart: boolean;
  phase: ContainerPhase;
  nsPid?: number;
  mntNsInode?: number;
  cgroupPath?: string;
  mounted: boolean;
  mountDir: string;
  imagePath: string;
  loopDevices: string[];
  hostname: string;
  sshEnabled: boolean;
  sshListenAddr: string;
  sshPort: number;
  primaryIpv4: string;
  hostBridge: boolean;
  hostRuntime: boolean;
  uptimeMs: number;
  restartCount: number;
  activeLeases: number;
  lastError?: string;
  disk: ContainerDiskStats;
}

export interface DaemonSummarySnapshot {
  pid: number;
  uid: number;
  uptimeMs: number;
  defaultContainer: string;
  totalContainers: number;
  runningContainers: number;
  activeTasks: number;
  sideveilConnected: boolean;
}

export interface StorageCleanupReport {
  drainedPids: number;
  unmountedPaths: number;
  detachedLoops: number;
  residualMounts: number;
  residualLoops: number;
}

export interface CompactResult {
  container: string;
  beforeBytes: number;
  afterBytes: number;
  trimmedBytes: number;
}

export interface FsckResult {
  container: string;
  exitCode: number;
  output: string;
}

export interface BackupStartResult {
  started: boolean;
  container: string;
  outputPath: string;
}

export interface MaintenanceDonePayload {
  container: string;
  op: string;
  ok: boolean;
  summary: string;
  error?: string;
}

export interface ContainerLogsResult {
  container: string;
  logPath: string;
  lines: string[];
}

export interface BindMount {
  hostPath: string;
  containerPath: string;
  readOnly: boolean;
}

export const L5_DIAG_MOUNTS: readonly BindMount[] = [
  { hostPath: '/sys/fs/pstore', containerPath: '/sys/fs/pstore', readOnly: true },
  { hostPath: '/sys/kernel/tracing', containerPath: '/sys/kernel/tracing', readOnly: true },
];

export interface ContainerConfig {
  enabled: boolean;
  autoStart: boolean;
  imagePath: string;
  mountDir: string;
  hostname: string;
  ssh: {
    enabled: boolean;
    listenAddr: string;
    port: number;
    keepaliveSecs: number;
  };
  oomScoreAdj?: number;
  hostBridge: boolean;
  hostRuntime: boolean;
  bindDataLocalTmp: boolean;
  extraCharDevs?: string[];
  extraMounts?: BindMount[];
  syncAndroidDns: boolean;
  fsckOnMount: boolean;
}

export interface ContainerConfigResult {
  container: string;
  config: ContainerConfig;
}

export interface ContainerConfigPatchParams {
  container?: string;
  enabled?: boolean;
  autoStart?: boolean;
  hostname?: string;
  sshPort?: number;
  hostBridge?: boolean;
  hostRuntime?: boolean;
  extraCharDevs?: string[];
  extraMounts?: BindMount[];
}

export interface SshKeysResult {
  container: string;
  keys: string[];
  added: number;
}

export interface TaskRunResult {
  taskId: string;
  container: string;
  pid: number;
}

export interface TaskSummary {
  taskId: string;
  container: string;
  pid: number;
  command: string[];
  running: boolean;
  exitCode?: number;
  startedAtMs: number;
}

export type TaskStream = 'stdout' | 'stderr';

export interface TaskOutputPayload {
  taskId: string;
  container: string;
  stream: TaskStream;
  chunk: string;
  seq: number;
}

export interface TaskLogsResult {
  summary: TaskSummary;
  entries: TaskOutputPayload[];
}

export interface TaskExitPayload {
  taskId: string;
  container: string;
  exitCode: number;
  signal?: number;
  timedOut: boolean;
}

export interface ContainerStateChangedPayload {
  container: string;
  previous: ContainerPhase;
  current: ContainerPhase;
  snapshot: ContainerStatusSnapshot;
}

export interface HudStateResult {
  enabled: boolean;
  clickThrough: boolean;
  isDark: boolean;
  sideveilConnected: boolean;
}
