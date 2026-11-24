/**
 * Multi-device desktop management types shared between Tauri IPC and React UI.
 */

export type DeviceKind = 'adb' | 'lan' | 'ssh' | 'mock';

export type AdbDeviceState =
  | 'device'
  | 'offline'
  | 'unauthorized'
  | 'recovery'
  | 'sideload'
  | string;

export interface AdbDevice {
  serial: string;
  state: AdbDeviceState;
  product?: string;
  model?: string;
  device?: string;
  transportId?: string;
  isWireless: boolean;
}

export interface AdbTunnel {
  serial: string;
  localPort: number;
  remotePort: number;
  baseUrl: string;
  token?: string;
}

export interface DeviceProfile {
  id: string;
  label: string;
  kind: DeviceKind;
  serial?: string;
  host: string;
  port: number;
  token?: string;
  sshUser?: string;
  sshPort?: number;
  model?: string;
}

export interface DeviceStoreFile {
  version: number;
  activeDeviceId?: string;
  devices: DeviceProfile[];
}

export interface EndpointProbeResult {
  reachable: boolean;
  authenticated: boolean;
  latencyMs: number;
  version?: string;
  lanEnabled?: boolean;
  error?: string;
}

export type NativeTerminalKind =
  | 'windows-terminal'
  | 'ghostty'
  | 'wezterm'
  | 'alacritty'
  | 'i-term2'
  | 'x-terminal-emulator';

export interface SshTarget {
  user: string;
  host: string;
  port: number;
  remotePath?: string;
  alias?: string;
}

export type NavViewId = 'containers' | 'services' | 'terminal' | 'install' | 'network' | 'logs';

export interface TerminalTabSpec {
  id: string;
  deviceId: string;
  containerId: string;
  title: string;
  sessionId?: string;
}
