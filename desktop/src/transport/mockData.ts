import type {
  ContainerConfig,
  ContainerServiceRuntime,
  ContainerStatusSnapshot,
  KernelCapabilitiesResult,
  WebStatusResult,
} from '../types/protocol.ts';
import { createInitialToolsState, type MockToolsState } from './mockToolsInitialState.ts';

export interface MockContainerRecord {
  snapshot: ContainerStatusSnapshot;
  config: ContainerConfig;
  services: ContainerServiceRuntime[];
  serviceLogs: Record<string, string[]>;
  sshKeys: string[];
  logs: string[];
  tools: MockToolsState;
}

export interface MockDeviceDataset {
  deviceLabel: string;
  capabilities: KernelCapabilitiesResult;
  webStatus: WebStatusResult;
  daemonLogs: string[];
  containers: Record<string, MockContainerRecord>;
}

const GB = 1024 * 1024 * 1024;

function makeTabletDataset(): MockDeviceDataset {
  const mainConfig: ContainerConfig = {
    enabled: true,
    autoStart: true,
    imagePath: '/data/local/strata/containers/main/rootfs.img',
    mountDir: '/data/local/strata/containers/main/mnt',
    hostname: 'strata-tablet',
    ssh: { enabled: true, listenAddr: '0.0.0.0', port: 2222, keepaliveSecs: 60 },
    oomScoreAdj: -300,
    hostBridge: true,
    hostRuntime: false,
    bindDataLocalTmp: true,
    extraCharDevs: ['uinput'],
    extraMounts: [],
    syncAndroidDns: true,
    dnsServers: ['223.5.5.5', '1.1.1.1'],
    fsckOnMount: false,
    noatime: true,
    initCommand: ['/bin/sh', '/etc/strata/init.sh'],
    services: [
      {
        id: 'rsshub',
        name: 'RSSHub 订阅聚合',
        command: ['node', '/opt/rsshub/dist/index.mjs'],
        workdir: '/opt/rsshub',
        env: { PORT: '1200', NODE_ENV: 'production' },
        autoStart: true,
        restartPolicy: 'on-failure',
        maxRestarts: 5,
        backoffMs: 2000,
      },
      {
        id: 'syncthing',
        name: 'Syncthing 文件同步',
        command: ['/usr/bin/syncthing', '--no-browser', '--gui-address=0.0.0.0:8384'],
        workdir: '/root',
        env: { HOME: '/root' },
        autoStart: true,
        restartPolicy: 'always',
        maxRestarts: 10,
        backoffMs: 3000,
      },
    ],
  };

  const devConfig: ContainerConfig = {
    ...mainConfig,
    autoStart: false,
    imagePath: '/data/local/strata/containers/arch-dev/rootfs.img',
    mountDir: '/data/local/strata/containers/arch-dev/mnt',
    hostname: 'arch-dev',
    ssh: { enabled: true, listenAddr: '0.0.0.0', port: 2223, keepaliveSecs: 60 },
    services: [],
  };

  return {
    deviceLabel: 'Demo Tablet',
    capabilities: {
      kernelRelease: '6.6.56-android16-8-gki-demo',
      pidNs: true,
      ipcNs: true,
      sysvIpc: false,
      userNs: true,
      netNs: true,
      overlayFs: true,
      fuseFs: true,
      cgroupV2: true,
      cgroupPids: true,
      cgroupBpf: true,
      ioUring: true,
      kvm: false,
      devtmpfs: true,
      systemdReady: false,
      dockerReady: false,
    },
    webStatus: {
      enabled: true,
      running: true,
      lanEnabled: true,
      listenAddr: '0.0.0.0',
      port: 7681,
      tokenPath: '/data/adb/strata/run/web.token',
      webrootAvailable: false,
      activePtySessions: 1,
      activeJobs: 0,
      lanUrls: ['http://192.0.2.10:7681'],
    },
    daemonLogs: [
      '[INFO] stratad 0.2.0 started on Android 16 GKI (Demo Tablet)',
      '[INFO] web API bound to 0.0.0.0:7681 (RFC1918 peer guard enabled)',
      '[INFO] container `main` booted in 412ms (nsPid=18422, loop=/dev/block/loop17)',
      '[INFO] service `main/rsshub` started (pid=18510, autoStart=true)',
      '[INFO] service `main/syncthing` started (pid=18534, autoStart=true)',
    ],
    containers: {
      main: {
        snapshot: {
          id: 'main',
          isDefault: true,
          enabled: true,
          autoStart: true,
          phase: 'running',
          nsPid: 18422,
          mntNsInode: 4026534112,
          cgroupPath: '/sys/fs/cgroup/strata/main',
          mounted: true,
          mountDir: mainConfig.mountDir,
          imagePath: mainConfig.imagePath,
          loopDevices: ['/dev/block/loop17'],
          hostname: 'strata-tablet',
          sshEnabled: true,
          sshListenAddr: '0.0.0.0',
          sshPort: 2222,
          primaryIpv4: '192.0.2.10',
          hostBridge: true,
          hostRuntime: false,
          uptimeMs: 14_820_000,
          restartCount: 0,
          activeLeases: 1,
          disk: {
            imageExists: true,
            imageAllocatedBytes: Math.round(5.4 * GB),
            imageVirtualBytes: 32 * GB,
            rootfsTotalBytes: 31 * GB,
            rootfsUsedBytes: Math.round(4.9 * GB),
            rootfsAvailableBytes: Math.round(26.1 * GB),
          },
        },
        config: mainConfig,
        services: [
          {
            id: 'rsshub',
            name: 'RSSHub 订阅聚合',
            status: 'running',
            pid: 18510,
            uptimeMs: 14_780_000,
            restartCount: 0,
            autoStart: true,
          },
          {
            id: 'syncthing',
            name: 'Syncthing 文件同步',
            status: 'running',
            pid: 18534,
            uptimeMs: 14_775_000,
            restartCount: 1,
            autoStart: true,
          },
        ],
        serviceLogs: {
          rsshub: [
            '2026-11-24 18:10:02 [rsshub] RSSHub listening on http://0.0.0.0:1200',
            '2026-11-24 21:40:15 [rsshub] GET /github/issue/DIYgod/RSSHub 200 184ms',
          ],
          syncthing: [
            '2026-11-24 18:10:04 [syncthing] My ID: STRATA7-DEMO-SYNC99',
            '2026-11-24 21:42:11 [syncthing] Folder "Notes" up to date (1428 files)',
          ],
        },
        sshKeys: [
          'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoDesktopWorkstationKey demo@workstation',
        ],
        logs: [
          '[init] mounted /proc /sys /dev/pts inside main container',
          '[init] dropbear listening on 0.0.0.0:2222 (public-key only)',
          '[init] synced Android DNS -> /etc/resolv.conf',
        ],
        tools: createInitialToolsState('main', true),
      },
      'arch-dev': {
        snapshot: {
          id: 'arch-dev',
          isDefault: false,
          enabled: true,
          autoStart: false,
          phase: 'stopped',
          mounted: false,
          mountDir: devConfig.mountDir,
          imagePath: devConfig.imagePath,
          loopDevices: [],
          hostname: 'arch-dev',
          sshEnabled: true,
          sshListenAddr: '0.0.0.0',
          sshPort: 2223,
          primaryIpv4: '192.0.2.10',
          hostBridge: true,
          hostRuntime: false,
          uptimeMs: 0,
          restartCount: 0,
          activeLeases: 0,
          disk: {
            imageExists: true,
            imageAllocatedBytes: Math.round(2.1 * GB),
            imageVirtualBytes: 24 * GB,
          },
        },
        config: devConfig,
        services: [],
        serviceLogs: {},
        sshKeys: [],
        logs: ['[init] container arch-dev stopped cleanly (0 residual mounts/loops)'],
        tools: createInitialToolsState('arch-dev', false),
      },
    },
  };
}

function makePhoneDataset(): MockDeviceDataset {
  const dataset = makeTabletDataset();
  dataset.deviceLabel = 'Demo Phone';
  dataset.capabilities.kernelRelease = '6.6.72-android16-gki-demo-phone';
  dataset.webStatus.lanUrls = ['http://192.0.2.20:7681'];
  const main = dataset.containers.main;
  if (main) {
    main.snapshot.hostname = 'strata-phone';
    main.snapshot.primaryIpv4 = '192.0.2.20';
    main.snapshot.uptimeMs = 3_420_000;
    main.snapshot.disk.imageAllocatedBytes = Math.round(3.1 * GB);
    main.config.hostname = 'strata-phone';
  }
  delete dataset.containers['arch-dev'];
  return dataset;
}

export function createMockDatasetForDevice(deviceId: string): MockDeviceDataset {
  return deviceId.includes('phone') ? makePhoneDataset() : makeTabletDataset();
}
