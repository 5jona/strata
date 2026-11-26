import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { collectFixtureFiles, validateFixtureFile, validateValue } from '@gojona/contracts-gen';
import {
  buildStrataRpcShellCommand,
  invokeStrataKsuRpc,
  readStrataKsuWebToken,
} from '../src/ksu/ksuStrataClient.ts';
import { createMockKsuStrataTransport } from '../src/ksu/mockKsuTransport.ts';
import { STRATA_MANIFEST } from '../src/manifest.ts';
import { createMockStrataTransport } from '../src/plugin/mockStrataBridge.ts';
import { hasL5DiagMounts, isL5DiagMount, toggleL5DiagMounts } from '../src/shared/passthrough.ts';
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

test('module/plugins/strata.manager/manifest.json has exact field-by-field parity with manifest.ts', () => {
  const manifestPath = path.join(REPO_ROOT, 'module/plugins/strata.manager/manifest.json');
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
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.WEB_STATUS),
    'WebStatusResult',
    'web.status',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.WEB_CONFIGURE, { lanEnabled: true }),
    'WebStatusResult',
    'web.configure',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.WEB_ROTATE_TOKEN),
    'WebRotateTokenResult',
    'web.rotateToken',
  );

  const services = await transport.invokePlugin(STRATA_METHODS.SERVICE_LIST, { container: 'main' });
  assert.equal(Array.isArray(services), true);
  assert.ok(services.length >= 1);
  for (const svc of services) {
    assertMatchesDef(svc, 'ContainerServiceRuntime', 'service.list[]');
  }
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.SERVICE_START, {
      container: 'main',
      serviceId: 'syncthing',
    }),
    'ContainerServiceRuntime',
    'service.start',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.SERVICE_STOP, {
      container: 'main',
      serviceId: 'syncthing',
    }),
    'ContainerServiceRuntime',
    'service.stop',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.SERVICE_LOGS, {
      container: 'main',
      serviceId: 'rsshub',
    }),
    'ServiceLogsResult',
    'service.logs',
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
    [STRATA_EVENTS.SERVICE_STATE_CHANGED]: 'ServiceStateChangedPayload',
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
  const rustCtlPath = path.join(REPO_ROOT, 'daemon/crates/strata-ctl/src/commands/config.rs');
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

test('standalone KSU strata rpc client quotes shell args safely and decodes typed results', async () => {
  const cmd = buildStrataRpcShellCommand('service.start', {
    container: 'main',
    serviceId: 'rsshub',
  });
  assert.match(cmd, /^'\/data\/adb\/modules\/strata\/bin\/strata' rpc 'service\.start' '/);

  const mockTransport = createMockKsuStrataTransport();
  const bridge = { exec: (c, opts) => mockTransport.exec(c, opts) };
  assertMatchesDef(
    await invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_STATUS),
    'DaemonSummarySnapshot',
    'ksu daemon.status',
  );
  assertMatchesDef(
    await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_STATUS),
    'WebStatusResult',
    'ksu web.status',
  );
  const tokenRes = await readStrataKsuWebToken(bridge);
  assert.ok(tokenRes.token.length >= 16);
});

test('version and versionCode stay aligned across module.prop, daemon, tools, WebUI, plugin, and Desktop', () => {
  const moduleProp = fs.readFileSync(path.join(REPO_ROOT, 'module/module.prop'), 'utf8');
  const versionMatch = moduleProp.match(/^version=v([0-9]+)\.([0-9]+)\.([0-9]+)$/m);
  const codeMatch = moduleProp.match(/^versionCode=([0-9]+)$/m);
  assert.ok(versionMatch, 'module/module.prop must declare version=vMAJOR.MINOR.PATCH');
  assert.ok(codeMatch, 'module/module.prop must declare numeric versionCode');

  const expectedVersion = `${versionMatch[1]}.${versionMatch[2]}.${versionMatch[3]}`;
  const expectedCode =
    Number(versionMatch[1]) * 10000 + Number(versionMatch[2]) * 100 + Number(versionMatch[3]);
  assert.equal(Number(codeMatch[1]), expectedCode);

  const daemonCargo = fs.readFileSync(path.join(REPO_ROOT, 'daemon/Cargo.toml'), 'utf8');
  const daemonCargoMatch = daemonCargo.match(
    /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
  );
  assert.ok(daemonCargoMatch, 'daemon/Cargo.toml must declare workspace.package.version');
  assert.equal(daemonCargoMatch[1], expectedVersion);

  const debugAgentCargo = fs.readFileSync(
    path.join(REPO_ROOT, 'tools/strata-debug-agent/Cargo.toml'),
    'utf8',
  );
  const debugAgentMatch = debugAgentCargo.match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  assert.ok(debugAgentMatch, 'tools/strata-debug-agent/Cargo.toml must declare package.version');
  assert.equal(debugAgentMatch[1], expectedVersion);

  const mcpCargo = fs.readFileSync(path.join(REPO_ROOT, 'tools/strata-mcp/Cargo.toml'), 'utf8');
  const mcpMatch = mcpCargo.match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  assert.ok(mcpMatch, 'tools/strata-mcp/Cargo.toml must declare package.version');
  assert.equal(mcpMatch[1], expectedVersion);

  const webuiPkg = JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'webui/package.json'), 'utf8'));
  const pluginManifest = JSON.parse(
    fs.readFileSync(path.join(REPO_ROOT, 'module/plugins/strata.manager/manifest.json'), 'utf8'),
  );
  assert.equal(webuiPkg.version, expectedVersion);
  assert.equal(pluginManifest.version, expectedVersion);
  assert.equal(STRATA_MANIFEST.version, expectedVersion);

  const desktopPkg = JSON.parse(
    fs.readFileSync(path.join(REPO_ROOT, 'desktop/package.json'), 'utf8'),
  );
  const desktopCargo = fs.readFileSync(path.join(REPO_ROOT, 'desktop/Cargo.toml'), 'utf8');
  const desktopCargoMatch = desktopCargo.match(
    /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
  );
  const tauriConf = JSON.parse(
    fs.readFileSync(path.join(REPO_ROOT, 'desktop/src-tauri/tauri.conf.json'), 'utf8'),
  );
  assert.ok(desktopCargoMatch, 'desktop/Cargo.toml must declare workspace.package.version');
  assert.equal(desktopPkg.version, expectedVersion);
  assert.equal(desktopCargoMatch[1], expectedVersion);
  assert.equal(tauriConf.version, expectedVersion);
});
