import { invoke } from '@tauri-apps/api/core';
import pkg from '../../package.json' with { type: 'json' };
import type {
  AdbDevice,
  AdbTunnel,
  DeviceProfile,
  DeviceStoreFile,
  EndpointProbeResult,
  NativeTerminalKind,
  PortForwardEntry,
  SshTarget,
} from '../types/device.ts';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

const STORAGE_KEY = 'strata-desktop:devices:v1';

/** Returns `true` when running inside the native Tauri v2 webview host. */
export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && window.__TAURI_INTERNALS__ !== undefined;
}

/** Browser-preview mock ADB devices when running `pnpm dev` outside Tauri. */
const BROWSER_MOCK_ADB_DEVICES: AdbDevice[] = [
  {
    serial: 'emulator-5554',
    state: 'device',
    product: 'demo_tablet',
    model: 'Demo_Tablet',
    device: 'generic_arm64_tab',
    transportId: '1',
    isWireless: false,
  },
  {
    serial: '192.0.2.20:5555',
    state: 'device',
    product: 'demo_phone',
    model: 'Demo_Phone',
    device: 'generic_arm64_phone',
    transportId: '2',
    isWireless: true,
  },
];

const browserMockForwards: PortForwardEntry[] = [];

function readBrowserStore(): DeviceStoreFile {
  if (typeof window === 'undefined') {
    return { version: 1, devices: [] };
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) {
      return JSON.parse(raw) as DeviceStoreFile;
    }
  } catch {
    // Ignore corrupt localStorage
  }
  return {
    version: 1,
    activeDeviceId: 'mock-tablet',
    devices: [
      {
        id: 'mock-tablet',
        label: 'Demo Tablet',
        kind: 'mock',
        serial: 'emulator-5554',
        host: '127.0.0.1',
        port: 7681,
        sshUser: 'root',
        sshPort: 2222,
        model: 'Demo Tablet',
      },
      {
        id: 'mock-phone',
        label: 'Demo Phone',
        kind: 'mock',
        serial: '192.0.2.20:5555',
        host: '192.0.2.20',
        port: 7681,
        sshUser: 'root',
        sshPort: 2222,
        model: 'Demo Phone',
      },
    ],
  };
}

function writeBrowserStore(store: DeviceStoreFile): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch {
    // Ignore storage quota errors
  }
}

/**
 * Facade over Tauri v2 IPC commands with automatic browser-preview fallback.
 */
export const tauriDeviceBridge = {
  async listAdbDevices(): Promise<AdbDevice[]> {
    if (isTauriRuntime()) {
      return invoke<AdbDevice[]>('list_adb_devices');
    }
    return [...BROWSER_MOCK_ADB_DEVICES];
  },

  async connectAdbDevice(serial: string, remotePort = 7681): Promise<AdbTunnel> {
    if (isTauriRuntime()) {
      return invoke<AdbTunnel>('connect_adb_device', { serial, remotePort });
    }
    const localPort = serial.includes('192.0.2.20') ? 17682 : 17681;
    return {
      serial,
      localPort,
      remotePort,
      baseUrl: `http://127.0.0.1:${localPort}`,
      token: 'mock-adb-bearer-token-preview',
    };
  },

  async disconnectDevice(serial: string): Promise<boolean> {
    if (isTauriRuntime()) {
      return invoke<boolean>('disconnect_device', { serial });
    }
    return true;
  },

  async forwardDevicePort(
    serial: string,
    remotePort: number,
    localPort?: number,
  ): Promise<PortForwardEntry> {
    if (isTauriRuntime()) {
      return invoke<PortForwardEntry>('forward_device_port', {
        serial,
        remotePort,
        localPort: localPort ?? null,
      });
    }
    const existing = browserMockForwards.find(
      (f) =>
        f.serial === serial &&
        f.remotePort === remotePort &&
        (!localPort || f.localPort === localPort),
    );
    if (existing) return { ...existing };
    const assignedLocal = localPort && localPort > 0 ? localPort : 20000 + remotePort;
    const entry: PortForwardEntry = {
      serial,
      localPort: assignedLocal,
      remotePort,
      localSpec: `tcp:${assignedLocal}`,
      remoteSpec: `tcp:${remotePort}`,
    };
    browserMockForwards.push(entry);
    return { ...entry };
  },

  async listPortForwards(serial?: string): Promise<PortForwardEntry[]> {
    if (isTauriRuntime()) {
      return invoke<PortForwardEntry[]>('list_port_forwards', { serial: serial ?? null });
    }
    return browserMockForwards.filter((f) => !serial || f.serial === serial).map((f) => ({ ...f }));
  },

  async removePortForward(serial: string, localPort: number): Promise<boolean> {
    if (isTauriRuntime()) {
      return invoke<boolean>('remove_port_forward', { serial, localPort });
    }
    const idx = browserMockForwards.findIndex(
      (f) => f.serial === serial && f.localPort === localPort,
    );
    if (idx >= 0) {
      browserMockForwards.splice(idx, 1);
      return true;
    }
    return false;
  },

  async loadSavedDevices(): Promise<DeviceStoreFile> {
    if (isTauriRuntime()) {
      return invoke<DeviceStoreFile>('load_saved_devices');
    }
    return readBrowserStore();
  },

  async saveDeviceProfile(profile: DeviceProfile): Promise<DeviceStoreFile> {
    if (isTauriRuntime()) {
      return invoke<DeviceStoreFile>('save_device_profile', { profile });
    }
    const store = readBrowserStore();
    const idx = store.devices.findIndex((d) => d.id === profile.id);
    if (idx >= 0) {
      store.devices[idx] = profile;
    } else {
      store.devices.push(profile);
    }
    writeBrowserStore(store);
    return store;
  },

  async deleteDeviceProfile(id: string): Promise<DeviceStoreFile> {
    if (isTauriRuntime()) {
      return invoke<DeviceStoreFile>('delete_device_profile', { id });
    }
    const store = readBrowserStore();
    store.devices = store.devices.filter((d) => d.id !== id);
    if (store.activeDeviceId === id) {
      store.activeDeviceId = store.devices[0]?.id;
    }
    writeBrowserStore(store);
    return store;
  },

  async probeEndpoint(host: string, port: number, token?: string): Promise<EndpointProbeResult> {
    if (isTauriRuntime()) {
      return invoke<EndpointProbeResult>('probe_device_endpoint', {
        host,
        port,
        token: token ?? null,
      });
    }
    return {
      reachable: true,
      authenticated: Boolean(token?.trim()),
      latencyMs: 6,
      version: `${pkg.version}-preview`,
      lanEnabled: host !== '127.0.0.1',
    };
  },

  async openNativeTerminal(target: SshTarget, preferred?: NativeTerminalKind): Promise<string> {
    if (isTauriRuntime()) {
      return invoke<string>('open_native_terminal', { target, preferred: preferred ?? null });
    }
    return `ssh -p ${target.port} ${target.user}@${target.host}`;
  },

  async openVsCodeRemote(target: SshTarget): Promise<string> {
    if (isTauriRuntime()) {
      return invoke<string>('open_vscode_remote', { target });
    }
    const path = target.remotePath ?? '/root';
    const uri = `vscode://vscode-remote/ssh-remote+${target.user}@${target.host}:${target.port}${path}`;
    if (typeof window !== 'undefined') {
      window.open(uri, '_self');
    }
    return uri;
  },
};
