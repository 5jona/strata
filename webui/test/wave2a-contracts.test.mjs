import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { validateValue } from '@gojona/contracts-gen';
import {
  buildStartDaemonShellCommand,
  invokeStrataKsuRpc,
  startStrataDaemonViaKsu,
} from '../src/ksu/ksuStrataClient.ts';
import { createMockKsuStrataTransport } from '../src/ksu/mockKsuTransport.ts';
import { createMockStrataTransport } from '../src/plugin/mockStrataBridge.ts';
import { STRATA_METHODS, STRATA_PATHS } from '../src/types.ts';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../..');
const ROOT_SCHEMA = JSON.parse(
  fs.readFileSync(path.join(REPO_ROOT, 'contracts/schemas/strata.schema.json'), 'utf8'),
);

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

test('Wave 2A KSU RPCs (daemon.capabilities, daemon.logs, sshKeyRemove, service.upsert, backupStart) match schemas', async () => {
  const transport = createMockStrataTransport();

  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_CAPABILITIES),
    'KernelCapabilitiesResult',
    'daemon.capabilities',
  );
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.DAEMON_LOGS, { lines: 20 }),
    'DaemonLogsResult',
    'daemon.logs',
  );

  await transport.invokePlugin(STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
    container: 'main',
    publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKeyCiContract ci@strata',
  });
  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE, {
      id: 'main',
      fingerprintOrKey: 'ci@strata',
    }),
    'SshKeyRemoveResult',
    'container.sshKeyRemove',
  );

  assertMatchesDef(
    await transport.invokePlugin(STRATA_METHODS.SERVICE_UPSERT, {
      container: 'main',
      spec: {
        id: 'syncthing',
        name: 'Syncthing 同步',
        command: ['/usr/bin/syncthing', '--no-browser'],
        workdir: '/root',
        env: { HOME: '/root' },
        autoStart: true,
        restartPolicy: 'always',
        maxRestarts: 10,
        backoffMs: 3000,
      },
    }),
    'ContainerServiceRuntime',
    'service.upsert',
  );

  const backupStart = await transport.invokePlugin(STRATA_METHODS.CONTAINER_BACKUP_START, {
    container: 'main',
  });
  assertMatchesDef(backupStart, 'BackupStartResult', 'container.backupStart');
  assert.equal(backupStart.outputPath.startsWith(`${STRATA_PATHS.DEFAULT_BACKUP_DIR}/`), true);
});

test('KSU daemon rescue shell command is non-blocking and supported by mock KSU transport', async () => {
  const cmd = buildStartDaemonShellCommand();
  assert.match(cmd, /service\.sh' --now >\/dev\/null 2>&1 <\/dev\/null &/);

  const mockTransport = createMockKsuStrataTransport();
  const bridge = { exec: (c, opts) => mockTransport.exec(c, opts) };
  await startStrataDaemonViaKsu(bridge);

  const caps = await invokeStrataKsuRpc(bridge, STRATA_METHODS.DAEMON_CAPABILITIES);
  assertMatchesDef(caps, 'KernelCapabilitiesResult', 'ksu daemon.capabilities');

  const setDefaultRes = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SET_DEFAULT, {
    id: 'main',
  });
  assertMatchesDef(setDefaultRes, 'ContainerSetDefaultResult', 'ksu container.setDefault');

  const createRes = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CREATE, {
    id: 'devbox',
    distroId: 'debian-12',
    sizeMb: 4096,
    enableSsh: true,
    autoStart: true,
    setDefault: true,
    hostBridge: true,
    l5DiagnosticMounts: false,
    syncAndroidDns: true,
    noatime: true,
    sshPublicKeys: [],
  });
  assertMatchesDef(createRes, 'ContainerJobStartResult', 'ksu container.create');

  const deleteRes = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_DELETE, {
    id: 'devbox',
    deleteImage: true,
    force: true,
  });
  assertMatchesDef(deleteRes, 'ContainerDeleteResult', 'ksu container.delete');
});
