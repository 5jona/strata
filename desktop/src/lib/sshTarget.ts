import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile, SshTarget } from '../types/device.ts';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';

/** Returns true when a container reports a non-loopback LAN IPv4 address. */
export function hasDirectLanIp(container: ContainerStatusSnapshot | null): boolean {
  const ip = container?.primaryIpv4?.trim();
  return Boolean(ip && ip !== '0.0.0.0' && ip !== '127.0.0.1');
}

/** Builds a synchronous preview SSH target without establishing an ADB forward tunnel. */
export function buildPreviewSshTarget(
  device: DeviceProfile | null,
  container: ContainerStatusSnapshot | null,
  remotePath = '/root',
): SshTarget {
  const host = hasDirectLanIp(container)
    ? (container?.primaryIpv4 ?? '127.0.0.1')
    : (device?.host ?? '127.0.0.1');
  return {
    user: device?.sshUser || 'root',
    host,
    port: container?.sshPort ?? device?.sshPort ?? 2222,
    remotePath,
    alias: `strata-${container?.id ?? 'main'}`,
  };
}

/**
 * Resolves the launch SSH target for VS Code Remote-SSH or an external terminal.
 * When `device.kind === 'adb'` and no direct LAN IP is available, automatically
 * establishes `adb forward` for `container.sshPort` and targets `127.0.0.1:<localPort>`.
 */
export async function resolveLaunchSshTarget(
  device: DeviceProfile | null,
  container: ContainerStatusSnapshot,
  remotePath = '/root',
): Promise<{ target: SshTarget; forwardedLocalPort?: number }> {
  if (device?.kind === 'adb' && device.serial && !hasDirectLanIp(container)) {
    const forward = await tauriDeviceBridge.forwardDevicePort(device.serial, container.sshPort);
    return {
      forwardedLocalPort: forward.localPort,
      target: {
        user: device.sshUser || 'root',
        host: '127.0.0.1',
        port: forward.localPort,
        remotePath,
        alias: `strata-${container.id}`,
      },
    };
  }
  return { target: buildPreviewSshTarget(device, container, remotePath) };
}
