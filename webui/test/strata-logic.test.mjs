import assert from 'node:assert/strict';
import test from 'node:test';
import { createMockStrataTransport } from '../src/lib/mockStrataBridge.ts';
import {
  buildSshCommand,
  buildTaskCommand,
  formatBytes,
  formatUptime,
  summarizeSshKey,
} from '../src/lib/strataFormat.ts';
import { STRATA_METHODS } from '../src/types.ts';

test('formatters produce stable human-readable output', () => {
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(2048), '2.0 KiB');
  assert.equal(formatBytes(5 * 1024 * 1024), '5.0 MiB');
  assert.equal(formatBytes(2 * 1024 * 1024 * 1024), '2.00 GiB');

  assert.equal(formatUptime(0), '未运行');
  assert.equal(formatUptime(42_000), '42s');
  assert.equal(formatUptime(125_000), '2m 5s');
  assert.equal(formatUptime(3_660_000), '1h 1m');
});

test('task command builders and SSH summaries handle shell/python and public keys', () => {
  assert.deepEqual(buildTaskCommand('shell', 'uname -a'), ['/bin/sh', '-c', 'uname -a']);
  assert.deepEqual(buildTaskCommand('python', 'print(1)'), ['/usr/bin/python3', '-c', 'print(1)']);
  assert.deepEqual(buildTaskCommand('shell', '   '), []);

  const key = summarizeSshKey(
    'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockPreviewKeyStrata dev@workstation',
  );
  assert.equal(key.algo, 'ssh-ed25519');
  assert.match(key.label, /^dev@workstation \(/);
});

test('mock Strata transport handles lifecycle, HUD, and streaming tasks', async () => {
  const transport = createMockStrataTransport();
  const list = await transport.invokePlugin(STRATA_METHODS.CONTAINER_LIST);
  assert.equal(Array.isArray(list), true);
  assert.equal(list[0].id, 'main');
  assert.equal(buildSshCommand(list[0]), 'ssh -p 2222 root@127.0.0.1');

  const hud = await transport.invokePlugin(STRATA_METHODS.HUD_SET, {
    enabled: true,
    clickThrough: true,
  });
  assert.equal(hud.enabled, true);
  assert.equal(hud.clickThrough, true);

  const receivedEvents = [];
  const unsub = transport.subscribe('plugin.event', (payload) => {
    receivedEvents.push(payload);
  });

  const started = await transport.invokePlugin(STRATA_METHODS.TASK_RUN, {
    container: 'main',
    command: ['/bin/sh', '-c', 'echo hi'],
  });
  assert.ok(started.taskId.startsWith('task-'));

  await new Promise((resolve) => setTimeout(resolve, 220));
  unsub();

  assert.ok(receivedEvents.some((e) => e.event === 'task.output'));
  assert.ok(receivedEvents.some((e) => e.event === 'task.exit'));

  const logs = await transport.invokePlugin(STRATA_METHODS.TASK_LOGS, {
    taskId: started.taskId,
  });
  assert.equal(logs.summary.running, false);
  assert.equal(logs.summary.exitCode, 0);
  assert.ok(logs.entries.length >= 2);
});
