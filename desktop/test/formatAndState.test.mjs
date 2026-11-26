import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  formatBytes,
  formatEnvLines,
  formatSshCommand,
  formatSshConfigSnippet,
  formatUptime,
  formatVsCodeRemoteUri,
  parseCommandArgv,
  parseEnvLines,
  summarizeSshKey,
  validateServiceSpec,
} from '../src/lib/format.ts';
import {
  getCustomMounts,
  hasL5DiagMounts,
  toggleCharDev,
  toggleL5DiagMounts,
} from '../src/lib/passthrough.ts';
import { resolveLaunchSshTarget } from '../src/lib/sshTarget.ts';
import { isTauriRuntime, tauriDeviceBridge } from '../src/transport/tauriBridge.ts';

test('formatting helpers format bytes, uptime, SSH, and VS Code URIs accurately', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(512 * 1024), '512 KiB');
  assert.equal(formatBytes(5.4 * 1024 * 1024 * 1024), '5.4 GiB');

  assert.equal(formatUptime(0), '未运行');
  assert.equal(formatUptime(45_000), '45秒');
  assert.equal(formatUptime(3_660_000), '1小时 1分');

  const target = {
    user: 'root',
    host: '192.0.2.10',
    port: 2222,
    remotePath: '/root/work',
    alias: 'strata-tablet',
  };
  assert.equal(formatSshCommand(target), 'ssh -p 2222 root@192.0.2.10');
  assert.equal(
    formatVsCodeRemoteUri(target),
    'vscode://vscode-remote/ssh-remote+root@192.0.2.10:2222/root/work',
  );
  const snippet = formatSshConfigSnippet(target);
  assert.match(snippet, /Host strata-tablet/);
  assert.match(snippet, /ControlMaster auto/);
});

test('service spec parsing and validation catch malformed fields', () => {
  assert.deepEqual(parseCommandArgv('node "/opt/my app/index.js" --port 1200'), [
    'node',
    '/opt/my app/index.js',
    '--port',
    '1200',
  ]);
  const env = parseEnvLines('# comment\nPORT=1200\nNODE_ENV=production\n');
  assert.deepEqual(env, { PORT: '1200', NODE_ENV: 'production' });
  assert.equal(formatEnvLines(env), 'PORT=1200\nNODE_ENV=production');

  assert.equal(
    validateServiceSpec({
      id: 'rsshub',
      name: 'RSSHub',
      command: ['node', 'index.js'],
      env: {},
      autoStart: true,
      restartPolicy: 'on-failure',
      maxRestarts: 5,
      backoffMs: 2000,
    }),
    null,
  );
  assert.ok(
    validateServiceSpec({
      id: 'bad/id',
      name: 'Bad',
      command: [],
      env: {},
      autoStart: false,
      restartPolicy: 'never',
      maxRestarts: 0,
      backoffMs: 50,
    }),
  );

  const keyInfo = summarizeSshKey('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest pochita@pc');
  assert.deepEqual(keyInfo, { algo: 'ED25519', label: 'pochita@pc' });
});

test('tauriDeviceBridge falls back cleanly outside native Tauri runtime', async () => {
  assert.equal(isTauriRuntime(), false);
  const adb = await tauriDeviceBridge.listAdbDevices();
  assert.ok(adb.length >= 2);
  const tunnel = await tauriDeviceBridge.connectAdbDevice(adb[0].serial, 7681);
  assert.match(tunnel.baseUrl, /^http:\/\/127\.0\.0\.1:\d+$/);

  const pkg = JSON.parse(
    fs.readFileSync(
      path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../package.json'),
      'utf8',
    ),
  );
  const probe = await tauriDeviceBridge.probeEndpoint('127.0.0.1', 7681, 'tok');
  assert.equal(probe.version, `${pkg.version}-preview`);

  const fwd = await tauriDeviceBridge.forwardDevicePort('emulator-5554', 2222);
  assert.ok(fwd.localPort > 0);
  const list = await tauriDeviceBridge.listPortForwards('emulator-5554');
  assert.ok(list.some((item) => item.localPort === fwd.localPort && item.remotePort === 2222));
  assert.equal(await tauriDeviceBridge.removePortForward('emulator-5554', fwd.localPort), true);
});

test('resolveLaunchSshTarget auto-forwards SSH port for USB ADB without LAN IP', async () => {
  const adbDevice = {
    id: 'adb-emulator-5554',
    label: 'USB Device',
    kind: 'adb',
    serial: 'emulator-5554',
    host: '127.0.0.1',
    port: 7681,
    sshUser: 'root',
    sshPort: 2222,
  };
  const noLanContainer = {
    id: 'main',
    isDefault: true,
    enabled: true,
    autoStart: true,
    phase: 'running',
    mounted: true,
    mountDir: '/data/local/strata/containers/main/mnt',
    imagePath: '/data/local/strata/containers/main/rootfs.img',
    loopDevices: [],
    hostname: 'main',
    sshEnabled: true,
    sshListenAddr: '0.0.0.0',
    sshPort: 2222,
    primaryIpv4: '0.0.0.0',
    hostBridge: true,
    hostRuntime: false,
    uptimeMs: 1000,
    restartCount: 0,
    activeLeases: 0,
    disk: { imageExists: true, imageAllocatedBytes: 0, imageVirtualBytes: 0 },
  };
  const resolved = await resolveLaunchSshTarget(adbDevice, noLanContainer);
  assert.equal(resolved.target.host, '127.0.0.1');
  assert.ok(resolved.forwardedLocalPort && resolved.forwardedLocalPort > 0);
  assert.equal(resolved.target.port, resolved.forwardedLocalPort);
});

test('passthrough helpers manage L5 diagnostic mounts and char devices without losing custom mounts', () => {
  const custom = [{ hostPath: '/sdcard/Download', containerPath: '/mnt/dl', readOnly: false }];
  const withL5 = toggleL5DiagMounts(custom, true);
  assert.equal(hasL5DiagMounts(withL5), true);
  assert.deepEqual(getCustomMounts(withL5), custom);

  const withoutL5 = toggleL5DiagMounts(withL5, false);
  assert.equal(hasL5DiagMounts(withoutL5), false);
  assert.deepEqual(withoutL5, custom);

  const devs = toggleCharDev(['uinput'], 'net/tun', true);
  assert.deepEqual(devs, ['uinput', 'net/tun']);
  assert.deepEqual(toggleCharDev(devs, 'uinput', false), ['net/tun']);
});
