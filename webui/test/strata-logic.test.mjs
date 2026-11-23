import assert from 'node:assert/strict';
import test from 'node:test';
import { createMockStrataTransport } from '../src/lib/mockStrataBridge.ts';
import {
  getCustomCharDevs,
  getCustomMounts,
  hasL5DiagMounts,
  toggleCharDev,
  toggleL5DiagMounts,
} from '../src/lib/passthrough.ts';
import {
  buildSshCommand,
  buildTaskCommand,
  formatBytes,
  formatMaintenanceToast,
  formatUptime,
  parseSshPort,
  summarizeSshKey,
} from '../src/lib/strataFormat.ts';
import { createStrataClient, invokeStrataRpc } from '../src/lib/strataRpc.ts';
import { L5_DIAG_MOUNTS, STRATA_EVENTS, STRATA_METHODS } from '../src/types.ts';

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

test('passthrough helpers merge/strip L5 mounts and char devs without clobbering custom entries', () => {
  const customMount = {
    hostPath: '/data/local/tmp/work',
    containerPath: '/mnt/work',
    readOnly: false,
  };
  assert.equal(hasL5DiagMounts([customMount]), false);

  const enabled = toggleL5DiagMounts([customMount], true);
  assert.equal(hasL5DiagMounts(enabled), true);
  assert.deepEqual(getCustomMounts(enabled), [customMount]);
  assert.equal(L5_DIAG_MOUNTS.length, 3);
  assert.ok(
    L5_DIAG_MOUNTS.some(
      (m) => m.hostPath === '/sys/fs/bpf' && m.containerPath === '/sys/fs/bpf' && m.readOnly,
    ),
  );
  assert.equal(enabled.length, 1 + L5_DIAG_MOUNTS.length);

  // Re-enabling must not duplicate L5 mounts.
  const enabledAgain = toggleL5DiagMounts(enabled, true);
  assert.equal(enabledAgain.length, enabled.length);

  const disabled = toggleL5DiagMounts(enabled, false);
  assert.equal(hasL5DiagMounts(disabled), false);
  assert.deepEqual(disabled, [customMount]);

  const devs1 = toggleCharDev(['binder'], 'uinput', true);
  assert.deepEqual(devs1, ['binder', 'uinput']);
  assert.deepEqual(getCustomCharDevs(devs1), ['binder']);
  const devs2 = toggleCharDev(devs1, 'uinput', false);
  assert.deepEqual(devs2, ['binder']);
});

test('mock Strata transport persists hostRuntime, extraCharDevs, and extraMounts in configPatch', async () => {
  const transport = createMockStrataTransport();
  const patched = await transport.invokePlugin(STRATA_METHODS.CONTAINER_CONFIG_PATCH, {
    container: 'main',
    hostRuntime: true,
    extraCharDevs: ['uinput', 'net/tun'],
    extraMounts: [...L5_DIAG_MOUNTS],
  });
  assert.equal(patched.config.hostRuntime, true);
  assert.deepEqual(patched.config.extraCharDevs, ['uinput', 'net/tun']);
  assert.deepEqual(patched.config.extraMounts, [...L5_DIAG_MOUNTS]);

  const fetched = await transport.invokePlugin(STRATA_METHODS.CONTAINER_CONFIG_GET, {
    container: 'main',
  });
  assert.equal(fetched.config.hostRuntime, true);
  assert.deepEqual(fetched.config.extraCharDevs, ['uinput', 'net/tun']);
  assert.deepEqual(fetched.config.extraMounts, [...L5_DIAG_MOUNTS]);
});

test('parseSshPort and formatMaintenanceToast validate ports and format op labels', () => {
  assert.equal(parseSshPort('2222'), 2222);
  assert.equal(parseSshPort(' 1 '), 1);
  assert.equal(parseSshPort('65535'), 65535);
  assert.equal(parseSshPort('0'), null);
  assert.equal(parseSshPort('65536'), null);
  assert.equal(parseSshPort('22abc'), null);
  assert.equal(parseSshPort(''), null);

  assert.deepEqual(
    formatMaintenanceToast({
      container: 'main',
      op: 'backup',
      ok: true,
      summary: '/data/local/tmp/a.img.gz',
    }),
    { title: '后台备份完成', tone: 'success', description: '/data/local/tmp/a.img.gz' },
  );
  assert.deepEqual(
    formatMaintenanceToast({
      container: 'main',
      op: 'fsck',
      ok: false,
      summary: 'e2fsck failed',
      error: 'exit=4',
    }),
    { title: '磁盘检查失败', tone: 'danger', description: 'exit=4' },
  );
  assert.deepEqual(
    formatMaintenanceToast({
      container: 'main',
      op: 'custom-op',
      ok: true,
      summary: 'done',
    }),
    { title: '维护任务 (custom-op)完成', tone: 'success', description: 'done' },
  );
});

test('createStrataClient and invokeStrataRpc delegate to typed plugin client cleanly', async () => {
  const calls = [];
  let subscribedEvent = null;
  let subscribedHandler = null;
  let unsubscribed = false;

  const stubBridge = {
    async invokePlugin(method, params, options) {
      calls.push({ method, params, options });
      if (method === STRATA_METHODS.DAEMON_PING) {
        return { pong: true, pid: 1, uptimeMs: 10 };
      }
      return { enabled: true, clickThrough: false, isDark: true, sideveilConnected: true };
    },
    onPluginEvent(event, handler) {
      subscribedEvent = event;
      subscribedHandler = handler;
      return () => {
        unsubscribed = true;
      };
    },
  };

  const client = createStrataClient(stubBridge, { defaultTimeoutMs: 5_000 });
  const ping = await client.invoke(STRATA_METHODS.DAEMON_PING);
  assert.equal(ping.pong, true);

  const hud = await invokeStrataRpc(
    stubBridge,
    STRATA_METHODS.HUD_SET,
    { enabled: true },
    { timeoutMs: 12_000 },
  );
  assert.equal(hud.enabled, true);

  const received = [];
  const unsub = client.onEvent(STRATA_EVENTS.TASK_EXIT, (payload) => {
    received.push(payload);
  });
  assert.equal(subscribedEvent, STRATA_EVENTS.TASK_EXIT);
  subscribedHandler({ taskId: 'task-1', exitCode: 0, timedOut: false, cancelled: false });
  unsub();

  assert.equal(unsubscribed, true);
  assert.equal(received.length, 1);
  assert.deepEqual(calls, [
    { method: STRATA_METHODS.DAEMON_PING, params: undefined, options: { timeoutMs: 5_000 } },
    { method: STRATA_METHODS.HUD_SET, params: { enabled: true }, options: { timeoutMs: 12_000 } },
  ]);
});
