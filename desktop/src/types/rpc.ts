import type {
  ContainerCloneParams,
  ContainerCloneResult,
  ContainerConfigPatchParams,
  ContainerConfigResult,
  ContainerCreateParams,
  ContainerDeleteParams,
  ContainerDeleteResult,
  ContainerJobProgressPayload,
  ContainerJobStartResult,
  ContainerPhase,
  ContainerResizeParams,
  ContainerResizeResult,
  ContainerServiceRuntime,
  ContainerServiceSpec,
  ContainerStatusSnapshot,
  DaemonLogsParams,
  DaemonLogsResult,
  DaemonSummarySnapshot,
  DistroCatalogParams,
  DistroCatalogResult,
  KernelCapabilitiesResult,
  PingResult,
  ServiceStatus,
  SshKeyRemoveResult,
  SshKeysResult,
  StorageCleanupReport,
  StrataConfig,
  WebConfigureParams,
  WebRotateTokenResult,
  WebStatusResult,
} from './protocol.ts';

export interface ContainerSelectorParams {
  container?: string;
}

export interface ContainerLogsParams {
  container?: string;
  lines?: number;
}

export interface ContainerLogsResult {
  container: string;
  logPath: string;
  lines: string[];
}

export interface SyncDnsResult {
  container: string;
  synced: boolean;
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

export interface BackupParams {
  container?: string;
  outputPath: string;
}

export interface BackupResult {
  container: string;
  outputPath: string;
  bytesWritten: number;
  restarted: boolean;
}

export interface SshKeyAddParams {
  container?: string;
  publicKey: string;
}

export interface SshKeyRemoveParams {
  id?: string;
  fingerprintOrKey: string;
}

export interface ContainerJobSelectorParams {
  jobId: string;
}

export interface ContainerJobCancelResult {
  jobId: string;
  cancelled: boolean;
}

export interface ServiceUpsertParams {
  container?: string;
  spec: ContainerServiceSpec;
}

export interface ServiceSelectorParams {
  container?: string;
  serviceId: string;
}

export interface ServiceLogsParams {
  container?: string;
  serviceId: string;
  limit?: number;
}

export interface ServiceLogsResult {
  container: string;
  serviceId: string;
  lines: string[];
}

export interface ServiceDeleteResult {
  container: string;
  serviceId: string;
  deleted: boolean;
}

export interface ContainerStateChangedPayload {
  container: string;
  previous: ContainerPhase;
  current: ContainerPhase;
  snapshot: ContainerStatusSnapshot;
}

export interface MaintenanceDonePayload {
  container: string;
  op: string;
  ok: boolean;
  summary: string;
  error?: string;
}

export interface ServiceStateChangedPayload {
  container: string;
  serviceId: string;
  prevStatus: ServiceStatus;
  status: ServiceStatus;
  pid?: number;
  restartCount: number;
  lastError?: string;
}

export const STRATA_METHODS = {
  DAEMON_PING: 'daemon.ping',
  DAEMON_STATUS: 'daemon.status',
  DAEMON_CAPABILITIES: 'daemon.capabilities',
  DAEMON_LOGS: 'daemon.logs',
  DAEMON_RELOAD_CONFIG: 'daemon.reloadConfig',
  WEB_STATUS: 'web.status',
  WEB_CONFIGURE: 'web.configure',
  WEB_ROTATE_TOKEN: 'web.rotateToken',
  DISTRO_CATALOG: 'distro.catalog',
  CONTAINER_LIST: 'container.list',
  CONTAINER_STATUS: 'container.status',
  CONTAINER_START: 'container.start',
  CONTAINER_STOP: 'container.stop',
  CONTAINER_RESTART: 'container.restart',
  CONTAINER_CREATE: 'container.create',
  CONTAINER_DELETE: 'container.delete',
  CONTAINER_RESIZE: 'container.resize',
  CONTAINER_CLONE: 'container.clone',
  CONTAINER_JOB_STATUS: 'container.jobStatus',
  CONTAINER_JOB_CANCEL: 'container.jobCancel',
  CONTAINER_LOGS: 'container.logs',
  CONTAINER_SYNC_DNS: 'container.syncDns',
  CONTAINER_COMPACT: 'container.compact',
  CONTAINER_FSCK: 'container.fsck',
  CONTAINER_BACKUP: 'container.backup',
  CONTAINER_CONFIG_GET: 'container.configGet',
  CONTAINER_CONFIG_PATCH: 'container.configPatch',
  CONTAINER_SSH_KEY_LIST: 'container.sshKeyList',
  CONTAINER_SSH_KEY_ADD: 'container.sshKeyAdd',
  CONTAINER_SSH_KEY_REMOVE: 'container.sshKeyRemove',
  SERVICE_LIST: 'service.list',
  SERVICE_UPSERT: 'service.upsert',
  SERVICE_DELETE: 'service.delete',
  SERVICE_START: 'service.start',
  SERVICE_STOP: 'service.stop',
  SERVICE_RESTART: 'service.restart',
  SERVICE_LOGS: 'service.logs',
} as const;

export const STRATA_EVENTS = {
  CONTAINER_STATE_CHANGED: 'container.stateChanged',
  CONTAINER_MAINTENANCE_DONE: 'container.maintenanceDone',
  CONTAINER_JOB_PROGRESS: 'container.job.progress',
  SERVICE_STATE_CHANGED: 'service.stateChanged',
} as const;

export interface StrataDesktopRpcMap {
  'daemon.ping': { params: undefined; result: PingResult };
  'daemon.status': { params: undefined; result: DaemonSummarySnapshot };
  'daemon.capabilities': { params: undefined; result: KernelCapabilitiesResult };
  'daemon.logs': { params: DaemonLogsParams; result: DaemonLogsResult };
  'daemon.reloadConfig': { params: undefined; result: StrataConfig };
  'web.status': { params: undefined; result: WebStatusResult };
  'web.configure': { params: WebConfigureParams; result: WebStatusResult };
  'web.rotateToken': { params: undefined; result: WebRotateTokenResult };
  'distro.catalog': { params: DistroCatalogParams; result: DistroCatalogResult };
  'container.list': { params: undefined; result: ContainerStatusSnapshot[] };
  'container.status': { params: ContainerSelectorParams; result: ContainerStatusSnapshot };
  'container.start': { params: ContainerSelectorParams; result: ContainerStatusSnapshot };
  'container.stop': { params: ContainerSelectorParams; result: StorageCleanupReport };
  'container.restart': { params: ContainerSelectorParams; result: ContainerStatusSnapshot };
  'container.create': { params: ContainerCreateParams; result: ContainerJobStartResult };
  'container.delete': { params: ContainerDeleteParams; result: ContainerDeleteResult };
  'container.resize': { params: ContainerResizeParams; result: ContainerResizeResult };
  'container.clone': { params: ContainerCloneParams; result: ContainerCloneResult };
  'container.jobStatus': {
    params: ContainerJobSelectorParams;
    result: ContainerJobProgressPayload;
  };
  'container.jobCancel': {
    params: ContainerJobSelectorParams;
    result: ContainerJobCancelResult;
  };
  'container.logs': { params: ContainerLogsParams; result: ContainerLogsResult };
  'container.syncDns': { params: ContainerSelectorParams; result: SyncDnsResult };
  'container.compact': { params: ContainerSelectorParams; result: CompactResult };
  'container.fsck': { params: ContainerSelectorParams; result: FsckResult };
  'container.backup': { params: BackupParams; result: BackupResult };
  'container.configGet': { params: ContainerSelectorParams; result: ContainerConfigResult };
  'container.configPatch': { params: ContainerConfigPatchParams; result: ContainerConfigResult };
  'container.sshKeyList': { params: ContainerSelectorParams; result: SshKeysResult };
  'container.sshKeyAdd': { params: SshKeyAddParams; result: SshKeysResult };
  'container.sshKeyRemove': { params: SshKeyRemoveParams; result: SshKeyRemoveResult };
  'service.list': { params: ContainerSelectorParams; result: ContainerServiceRuntime[] };
  'service.upsert': { params: ServiceUpsertParams; result: ContainerServiceRuntime };
  'service.delete': { params: ServiceSelectorParams; result: ServiceDeleteResult };
  'service.start': { params: ServiceSelectorParams; result: ContainerServiceRuntime };
  'service.stop': { params: ServiceSelectorParams; result: ContainerServiceRuntime };
  'service.restart': { params: ServiceSelectorParams; result: ContainerServiceRuntime };
  'service.logs': { params: ServiceLogsParams; result: ServiceLogsResult };
}

export type StrataDesktopMethod = keyof StrataDesktopRpcMap;

export interface StrataDesktopEventMap {
  'container.stateChanged': ContainerStateChangedPayload;
  'container.maintenanceDone': MaintenanceDonePayload;
  'container.job.progress': ContainerJobProgressPayload;
  'service.stateChanged': ServiceStateChangedPayload;
}

export type StrataDesktopEventName = keyof StrataDesktopEventMap;

export type StrataEventEnvelope = {
  [K in StrataDesktopEventName]: {
    event: K;
    payload: StrataDesktopEventMap[K];
  };
}[StrataDesktopEventName];

export class StrataRpcError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(message);
    this.name = 'StrataRpcError';
    this.code = code;
  }
}
