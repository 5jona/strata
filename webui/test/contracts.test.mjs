import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { collectFixtureFiles, validateFixtureFile, validateValue } from '@gojona/contracts-gen';
import { createMockStrataTransport } from '../src/lib/mockStrataBridge.ts';
import { hasL5DiagMounts, isL5DiagMount, toggleL5DiagMounts } from '../src/lib/passthrough.ts';
import { STRATA_MANIFEST } from '../src/manifest.ts';
import { L5_DIAG_MOUNTS, L5_DIAG_PATHS, STRATA_EVENTS, STRATA_METHODS } from '../src/types.ts';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../..');
const SCHEMA_PATH = path.join(REPO_ROOT, 'contracts/schemas/strata.schema.json');
const FIXTURES_DIR = path.join(REPO_ROOT, 'contracts/fixtures');
const ROOT_SCHEMA = JSON.parse(fs.readFileSync(SCHEMA_PATH, 'utf8'));

function assertMatchesDef(value, defName, label) {
  const schema = ROOT_SCHEMA.$defs?.[defName];
  assert.ok(schema, `Missing schema $defs.${defName}`);
  const errors = validateValue(value, schema, ROOT_SCHEMA, '$');
  assert.deepEqual(
    errors,
    [],
    `${label} failed schema validation against ${defName}:\n${errors.join('\n')}`,
  );
}

test('plugins/strata.manager/manifest.json has exact field-by-field parity with manifest.ts', () => {
  const manifestPath = path.join(REPO_ROOT, 'plugins/strata.manager/manifest.json');
  const manifestJson = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  assert.deepEqual(manifestJson, STRATA_MANIFEST);
});

test('all golden JSON fixtures validate against contracts/schemas/strata.schema.json', () => {
  const files = collectFixtureFiles([FIXTURES_DIR]);
  assert.ok(files.length >= 10, `Expected at least 10 golden fixtures, found ${files.length}`);
  for (const file of files) {
    const result = validateFixtureFile(ROOT_SCHEMA, file);
    assert.equal(
      result.ok,
      true,
      `Fixture ${path.relative(REPO_ROOT, file)} [${result.defName}] failed:\n${result.errors.join('\n')}`,
    );
  }
});

test('STRATA_EVENTS excludes SideVeil Hub plugin.eventLagged and matches contract spec', () => {
  const spec = JSON.parse(
    fs.readFileSync(path.join(REPO_ROOT, 'contracts/strata.contract.json'), 'utf8'),
  );
  const specEvents = Object.values(spec.events)
    .map((e) => e.event)
    .sort();
  const tsEvents = Object.values(STRATA_EVENTS).sort();
  assert.deepEqual(tsEvents, specEvents);
  assert.equal(tsEvents.includes('plugin.eventLagged'), false);
});

test('mockStrataBridge RPC responses and pushed events conform to generated JSON Schemas', async () => {
  const transport = createMockStrataTransport();
  const capturedEvents = [];
  const unsub = transport.subscribe('plugin.event', (evt) => {
    capturedEvents.push(evt);
  });

  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_PING),
    'PingResult',
    'daemon.ping',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_HEALTH),
    'DaemonHealthResult',
    'daemon.health',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_STATUS),
    'DaemonSummarySnapshot',
    'daemon.status',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_RELOAD_CONFIG),
    'StrataConfig',
    'daemon.reloadConfig',
  );

  const list = await transport.invokePlugin(STRATA_METHODS.CONTAINER_LIST);
  assert.equal(Array.isArray(list), true);
  assert.ok(list.length > 0);
  for (const item of list) {
    assertMatchesDef(item, 'ContainerStatusSnapshot', 'container.list[]');
  }

  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_STATUS, { container: 'main' }),
    'ContainerStatusSnapshot',
    'container.status',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_STOP, { container: 'main' }),
    'StorageCleanupReport',
    'container.stop',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_START, { container: 'main' }),
    'ContainerStatusSnapshot',
    'container.start',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_RESTART, { container: 'main' }),
    'ContainerStatusSnapshot',
    'container.restart',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_SYNC_DNS, { container: 'main' }),
    'SyncDnsResult',
    'container.syncDns',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_COMPACT, { container: 'main' }),
    'CompactResult',
    'container.compact',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_FSCK, { container: 'main' }),
    'FsckResult',
    'container.fsck',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_LOGS, { container: 'main', lines: 20 }),
    'ContainerLogsResult',
    'container.logs',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_CONFIG_GET, { container: 'main' }),
    'ContainerConfigResult',
    'container.configGet',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_CONFIG_PATCH, {
      container: 'main',
      hostname: 'strata-ci',
      imagePath: '/data/local/strata/containers/main/rootfs.img',
      mountDir: '/data/local/strata/containers/main/mnt',
      extraMounts: [...L5_DIAG_MOUNTS],
    }),
    'ContainerConfigResult',
    'container.configPatch',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_SSH_KEY_LIST, { container: 'main' }),
    'SshKeysResult',
    'container.sshKeyList',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
      container: 'main',
      publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKeyCiContract ci@strata',
    }),
    'SshKeysResult',
    'container.sshKeyAdd',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.HUD_STATE),
    'HudStateResult',
    'hud.state',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.HUD_SET, { enabled: true, isDark: true }),
    'HudStateResult',
    'hud.set',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.EVENTS_SUBSCRIBE),
    'EventsSubscribeResult',
    'events.subscribe',
  );

  const backupStart = await transport.invokePlugin(STRATA_METHODS.CONTAINER_BACKUP_START, {
    container: 'main',
    outputPath: '/data/local/tmp/strata-ci.img.gz',
  });
  assertMatchesDef(backupStart, 'BackupStartResult', 'container.backupStart');

  const taskRun = await transport.invokePlugin(STRATA_METHODS.TASK_RUN, {
    container: 'main',
    command: ['/bin/sh', '-c', 'echo contract'],
  });
  assertMatchesDef(taskRun, 'TaskRunResult', 'task.run');

  await new Promise((resolve) => setTimeout(resolve, 360));
  unsub();

  const taskList = await transport.invokePlugin(STRATA_METHODS.TASK_LIST);
  assert.equal(Array.isArray(taskList), true);
  assert.ok(taskList.length >= 1);
  for (const summary of taskList) {
    assertMatchesDef(summary, 'TaskSummary', 'task.list[]');
  }
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.TASK_LOGS, { taskId: taskRun.taskId }),
    'TaskLogsResult',
    'task.logs',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.TASK_CANCEL, { taskId: taskRun.taskId }),
    'TaskCancelResult',
    'task.cancel',
  );

  const eventDefs = {
    [STRATA_EVENTS.CONTAINER_STATE_CHANGED]: 'ContainerStateChangedPayload',
    [STRATA_EVENTS.CONTAINER_MAINTENANCE_DONE]: 'MaintenanceDonePayload',
    [STRATA_EVENTS.TASK_OUTPUT]: 'TaskOutputPayload',
    [STRATA_EVENTS.TASK_EXIT]: 'TaskExitPayload',
  };
  for (const [eventName, defName] of Object.entries(eventDefs)) {
    const matched = capturedEvents.filter((e) => e.event === eventName);
    assert.ok(matched.length >= 1, `Expected mock bridge to emit at least one ${eventName} event`);
    for (const item of matched) {
      assertMatchesDef(item.payload, defName, `event:${eventName}`);
    }
  }
});

test('L5_DIAG_MOUNTS passthrough matching aligns between passthrough.ts and Rust strata-ctl', () => {
  const rustCtlPath = path.join(REPO_ROOT, 'crates/strata-ctl/src/commands/config.rs');
  const rustCtlSource = fs.readFileSync(rustCtlPath, 'utf8');
  assert.match(
    rustCtlSource,
    /!L5_DIAG_MOUNT_PATHS\.contains\(&bm\.container_path\.as_str\(\)\)/,
    'Rust strata-ctl merge_l5_diag_mounts must filter by container_path against L5_DIAG_MOUNT_PATHS',
  );

  const expectedPaths = Object.values(L5_DIAG_PATHS);
  assert.deepEqual(
    L5_DIAG_MOUNTS.map((m) => [m.hostPath, m.containerPath, m.readOnly]),
    expectedPaths.map((p) => [p, p, true]),
  );

  // An alternate hostPath targeting a canonical L5 containerPath must be treated as an L5 mount
  // (matching Rust strata-ctl container_path filtering) so toggling never produces duplicates.
  const overridden = {
    hostPath: '/tmp/custom-pstore',
    containerPath: L5_DIAG_PATHS.PSTORE,
    readOnly: false,
  };
  const custom = { hostPath: '/data/local/tmp/work', containerPath: '/mnt/work', readOnly: false };
  assert.equal(isL5DiagMount(overridden), true);
  assert.equal(hasL5DiagMounts([overridden, custom]), false);

  const enabled = toggleL5DiagMounts([overridden, custom], true);
  assert.equal(enabled.length, 1 + L5_DIAG_MOUNTS.length);
  assert.deepEqual(enabled[0], custom);
  assert.equal(hasL5DiagMounts(enabled), true);

  const disabled = toggleL5DiagMounts(enabled, false);
  assert.deepEqual(disabled, [custom]);
});
