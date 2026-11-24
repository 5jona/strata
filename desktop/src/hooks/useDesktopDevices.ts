import { useToast } from '@gojona/ui';
import { useCallback, useEffect, useRef, useState } from 'react';
import { HttpWsTransport, type StrataTransport } from '../transport/httpWsTransport.ts';
import { MockStrataTransport } from '../transport/mockTransport.ts';
import { isTauriRuntime, tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { AdbDevice, DeviceProfile } from '../types/device.ts';

export type ConnectionPhase = 'connecting' | 'connected' | 'disconnected' | 'error';

export interface UseDesktopDevicesResult {
  devices: DeviceProfile[];
  adbDevices: AdbDevice[];
  activeDevice: DeviceProfile | null;
  transport: StrataTransport | null;
  connectionPhase: ConnectionPhase;
  connectionError: string | null;
  scanningAdb: boolean;
  selectDevice: (id: string) => void;
  scanAdbDevices: () => Promise<void>;
  connectAdbDevice: (adb: AdbDevice, remotePort?: number) => Promise<void>;
  saveManualDevice: (profile: DeviceProfile) => Promise<void>;
  removeDevice: (id: string) => Promise<void>;
}

/**
 * Manages multi-device registry (ADB USB/Wireless + manual LAN/SSH + preview mock),
 * automatic `adb forward` tunnel setup, and active `StrataTransport` lifecycle.
 */
export function useDesktopDevices(): UseDesktopDevicesResult {
  const { pushToast } = useToast();
  const [devices, setDevices] = useState<DeviceProfile[]>([]);
  const [adbDevices, setAdbDevices] = useState<AdbDevice[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [transport, setTransport] = useState<StrataTransport | null>(null);
  const [connectionPhase, setConnectionPhase] = useState<ConnectionPhase>('connecting');
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [scanningAdb, setScanningAdb] = useState(false);
  const transportRef = useRef<StrataTransport | null>(null);

  const scanAdbDevices = useCallback(async () => {
    setScanningAdb(true);
    try {
      const list = await tauriDeviceBridge.listAdbDevices();
      setAdbDevices(list);
    } catch (err) {
      pushToast(`ADB 扫描失败: ${err instanceof Error ? err.message : String(err)}`, 'warning');
    } finally {
      setScanningAdb(false);
    }
  }, [pushToast]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const store = await tauriDeviceBridge.loadSavedDevices();
        if (cancelled) return;
        setDevices(store.devices);
        const initialId = store.activeDeviceId ?? store.devices[0]?.id ?? null;
        setActiveId(initialId);
      } catch (err) {
        if (!cancelled) {
          setConnectionPhase('error');
          setConnectionError(err instanceof Error ? err.message : String(err));
        }
      }
      if (!cancelled) {
        await scanAdbDevices();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [scanAdbDevices]);

  const activeDevice = devices.find((d) => d.id === activeId) ?? null;

  useEffect(() => {
    if (!activeDevice) {
      transportRef.current?.dispose();
      transportRef.current = null;
      setTransport(null);
      setConnectionPhase('disconnected');
      setConnectionError(null);
      return;
    }

    let cancelled = false;
    setConnectionPhase('connecting');
    setConnectionError(null);

    void (async () => {
      try {
        let nextTransport: StrataTransport;
        if (activeDevice.kind === 'mock') {
          nextTransport = new MockStrataTransport(activeDevice.id);
        } else if (activeDevice.kind === 'adb' && activeDevice.serial) {
          if (!isTauriRuntime()) {
            nextTransport = new MockStrataTransport(activeDevice.id);
          } else {
            const tunnel = await tauriDeviceBridge.connectAdbDevice(
              activeDevice.serial,
              activeDevice.port,
            );
            const effectiveToken = activeDevice.token || tunnel.token || '';
            nextTransport = new HttpWsTransport(tunnel.baseUrl, effectiveToken);
          }
        } else {
          const baseUrl = `http://${activeDevice.host}:${activeDevice.port}`;
          nextTransport = new HttpWsTransport(baseUrl, activeDevice.token ?? '');
        }

        if (cancelled) {
          nextTransport.dispose();
          return;
        }
        await nextTransport.invoke('daemon.ping');
        if (cancelled) {
          nextTransport.dispose();
          return;
        }
        transportRef.current?.dispose();
        transportRef.current = nextTransport;
        setTransport(nextTransport);
        setConnectionPhase('connected');
      } catch (err) {
        if (!cancelled) {
          setConnectionPhase('error');
          setConnectionError(err instanceof Error ? err.message : String(err));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeDevice]);

  useEffect(() => {
    return () => {
      transportRef.current?.dispose();
      transportRef.current = null;
    };
  }, []);

  const connectAdbDevice = useCallback(
    async (adb: AdbDevice, remotePort = 7681) => {
      try {
        const tunnel = await tauriDeviceBridge.connectAdbDevice(adb.serial, remotePort);
        const safeId = `adb-${adb.serial.replace(/[^a-zA-Z0-9._-]/g, '-')}`;
        const profile: DeviceProfile = {
          id: safeId,
          label: adb.model ? `${adb.model} (${adb.isWireless ? '无线 ADB' : 'USB'})` : adb.serial,
          kind: isTauriRuntime() ? 'adb' : 'mock',
          serial: adb.serial,
          host: '127.0.0.1',
          port: remotePort,
          ...(tunnel.token ? { token: tunnel.token } : {}),
          sshUser: 'root',
          sshPort: 2222,
          ...(adb.model ? { model: adb.model } : {}),
        };
        const updated = await tauriDeviceBridge.saveDeviceProfile(profile);
        setDevices(updated.devices);
        setActiveId(profile.id);
        pushToast(
          tunnel.token
            ? `已连接 ${profile.label}（已自动获取 Bearer Token）`
            : `已建立 ADB 转发，请在设备配置中补充 Bearer Token`,
          tunnel.token ? 'success' : 'warning',
        );
      } catch (err) {
        pushToast(
          `连接 ADB 设备失败: ${err instanceof Error ? err.message : String(err)}`,
          'danger',
        );
      }
    },
    [pushToast],
  );

  const saveManualDevice = useCallback(
    async (profile: DeviceProfile) => {
      const updated = await tauriDeviceBridge.saveDeviceProfile(profile);
      setDevices(updated.devices);
      setActiveId(profile.id);
      pushToast(`已保存设备 ${profile.label}`, 'success');
    },
    [pushToast],
  );

  const removeDevice = useCallback(
    async (id: string) => {
      const target = devices.find((d) => d.id === id);
      if (target?.kind === 'adb' && target.serial) {
        await tauriDeviceBridge.disconnectDevice(target.serial).catch(() => {});
      }
      const updated = await tauriDeviceBridge.deleteDeviceProfile(id);
      setDevices(updated.devices);
      if (activeId === id) {
        setActiveId(updated.devices[0]?.id ?? null);
      }
      pushToast('已移除设备配置', 'info');
    },
    [activeId, devices, pushToast],
  );

  return {
    devices,
    adbDevices,
    activeDevice,
    transport,
    connectionPhase,
    connectionError,
    scanningAdb,
    selectDevice: setActiveId,
    scanAdbDevices,
    connectAdbDevice,
    saveManualDevice,
    removeDevice,
  };
}
