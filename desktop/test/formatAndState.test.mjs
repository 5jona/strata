import assert from 'node:assert/strict';
import test from 'node:test';
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
import { isTauriRuntime, tauriDeviceBridge } from '../src/transport/tauriBridge.ts';

test('formatting helpers format bytes, uptime, SSH, and VS Code URIs accurately', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(512 * 1024), '512 KiB');
  assert.equal(formatBytes(5.4 * 1024 * 1024 * 1024), '5.4 GiB');

  assert.equal(formatUptime(0), '未运行');
  assert.equal(formatUptime(45_000), '45秒');
  assert.equal(formatUptime(3_660_000), '1时 1分');

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
});
