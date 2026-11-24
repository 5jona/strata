/**
 * Wire DTOs aligned 1:1 with `contracts/strata.contract.json`.
 */

export type ServiceRestartPolicy = 'on-failure' | 'always' | 'never';
export type ServiceStatus = 'stopped' | 'running' | 'crashed' | 'circuit-open';
export type CapProfile = 'isolated' | 'host-bridge' | 'full-host-runtime';
export type ContainerPhase = 'stopped' | 'starting' | 'running' | 'stopping' | 'failed';
export type DistroMirror = 'tuna' | 'official' | 'custom';
export type ContainerJobKind = 'create' | 'import' | 'clone' | 'resize' | 'export';
export type ContainerJobState = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface WebConsoleConfig {
  enabled: boolean;
  lanEnabled: boolean;
  port: number;
}

export interface SshConfig {
  enabled: boolean;
  listenAddr: string;
  port: number;
  keepaliveSecs: number;
}

export interface BindMount {
  hostPath: string;
  containerPath: string;
  readOnly: boolean;
}

export interface ContainerServiceSpec {
  id: string;
  name: string;
  command: string[];
  workdir?: string;
  env: Record<string, string>;
  autoStart: boolean;
  restartPolicy: ServiceRestartPolicy;
  maxRestarts: number;
  backoffMs: number;
}

export interface ContainerServiceRuntime {
  id: string;
  name: string;
  status: ServiceStatus;
  pid?: number;
  uptimeMs: number;
  restartCount: number;
  lastError?: string;
  autoStart: boolean;
}

export interface ContainerConfig {
  enabled: boolean;
  autoStart: boolean;
  imagePath: string;
  mountDir: string;
  hostname: string;
  ssh: SshConfig;
  oomScoreAdj?: number;
  hostBridge: boolean;
  hostRuntime: boolean;
  bindDataLocalTmp: boolean;
  extraCharDevs: string[];
  extraMounts: BindMount[];
  syncAndroidDns: boolean;
  dnsServers: string[];
  fsckOnMount: boolean;
  noatime: boolean;
  initCommand: string[];
  services: ContainerServiceSpec[];
}

export interface StrataConfig {
  version: number;
  defaultContainer: string;
  web: WebConsoleConfig;
  containers: Record<string, ContainerConfig>;
}

export interface ContainerConfigPatchParams {
  container?: string;
  enabled?: boolean;
  autoStart?: boolean;
  imagePath?: string;
  mountDir?: string;
  hostname?: string;
  sshPort?: number;
  hostBridge?: boolean;
  hostRuntime?: boolean;
  noatime?: boolean;
  extraCharDevs?: string[];
  extraMounts?: BindMount[];
}

export interface ContainerConfigResult {
  container: string;
  config: ContainerConfig;
}

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

export interface PingResult {
  pong: boolean;
  pid: number;
  uptimeMs: number;
}

export interface KernelCapabilitiesResult {
  kernelRelease: string;
  pidNs: boolean;
  ipcNs: boolean;
  sysvIpc: boolean;
  userNs: boolean;
  netNs: boolean;
  overlayFs: boolean;
  fuseFs: boolean;
  cgroupV2: boolean;
  cgroupPids: boolean;
  cgroupBpf: boolean;
  ioUring: boolean;
  kvm: boolean;
  devtmpfs: boolean;
  systemdReady: boolean;
  dockerReady: boolean;
}

export interface DaemonLogsParams {
  lines?: number;
}

export interface DaemonLogsResult {
  logPath: string;
  lines: string[];
}

export interface WebStatusResult {
  enabled: boolean;
  running: boolean;
  lanEnabled: boolean;
  listenAddr: string;
  port: number;
  tokenPath: string;
  webrootAvailable: boolean;
  activePtySessions: number;
  activeJobs: number;
  lanUrls: string[];
}

export interface WebConfigureParams {
  enabled?: boolean;
  lanEnabled?: boolean;
  port?: number;
}

export interface WebRotateTokenResult {
  token: string;
  tokenPath: string;
  sessionsRevoked: number;
}

export interface DistroCatalogParams {
  mirror?: DistroMirror;
  customUrl?: string;
  refresh: boolean;
}

export interface DistroEntry {
  id: string;
  os: string;
  release: string;
  releaseTitle: string;
  arch: string;
  variant: string;
  version: string;
  sizeBytes: number;
  sha256: string;
  downloadUrl: string;
}

export interface DistroCatalogResult {
  mirror: DistroMirror;
  baseUrl: string;
  cached: boolean;
  fetchedAtMs: number;
  entries: DistroEntry[];
}

export interface ContainerCreateParams {
  id: string;
  distroId?: string;
  mirror?: DistroMirror;
  customUrl?: string;
  sourceUrl?: string;
  expectedSha256?: string;
  sizeMb: number;
  hostname?: string;
  sshPort?: number;
  sshPublicKeys: string[];
  autoStart: boolean;
  hostBridge: boolean;
  noatime: boolean;
}

export interface ContainerJobStartResult {
  jobId: string;
  container: string;
  kind: ContainerJobKind;
}

export interface ContainerJobProgressPayload {
  jobId: string;
  container: string;
  kind: ContainerJobKind;
  state: ContainerJobState;
  stage: string;
  bytesDone: number;
  bytesTotal?: number;
  percent?: number;
  message?: string;
  error?: string;
  startedAtMs: number;
  updatedAtMs: number;
}

export interface ContainerDeleteParams {
  id: string;
  deleteImage: boolean;
  force: boolean;
}

export interface StorageCleanupReport {
  drainedPids: number;
  unmountedPaths: number;
  detachedLoops: number;
  residualMounts: number;
  residualLoops: number;
}

export interface ContainerDeleteResult {
  container: string;
  imageDeleted: boolean;
  cleanup: StorageCleanupReport;
  newDefaultContainer?: string;
}

export interface ContainerResizeParams {
  id?: string;
  sizeMb: number;
}

export interface ContainerResizeResult {
  container: string;
  previousBytes: number;
  newBytes: number;
  restarted: boolean;
}

export interface ContainerCloneParams {
  sourceId: string;
  targetId: string;
  sshPort?: number;
  hostname?: string;
}

export interface ContainerCloneResult {
  sourceId: string;
  targetId: string;
  imagePath: string;
  copiedBytes: number;
  virtualBytes: number;
}

export interface SshKeysResult {
  container: string;
  keys: string[];
  added: number;
}

export interface SshKeyRemoveResult {
  container: string;
  removed: number;
  keys: string[];
}
