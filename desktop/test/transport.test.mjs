import assert from 'node:assert/strict';
import test from 'node:test';
import { HttpWsTransport } from '../src/transport/httpWsTransport.ts';
import { MockStrataTransport } from '../src/transport/mockTransport.ts';
import { STRATA_EVENTS, STRATA_METHODS, StrataRpcError } from '../src/types/rpc.ts';

test('HttpWsTransport attaches Authorization Bearer header and decodes JSON-RPC 2.0 responses', async () => {
  let capturedAuth = '';
  let capturedBody = '';
  const fakeFetch = async (_url, init) => {
    const headers = init?.headers ?? {};
    capturedAuth = headers.Authorization ?? '';
    capturedBody = String(init?.body ?? '');
    return new Response(
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        result: { pong: true, pid: 777, uptimeMs: 1234 },
      }),
      { status: 200, headers: { 'Content-Type': 'application/json' } },
    );
  };

  const transport = new HttpWsTransport('http://127.0.0.1:7681/', 'test-secret-token', fakeFetch);
  const res = await transport.invoke(STRATA_METHODS.DAEMON_PING);
  assert.equal(capturedAuth, 'Bearer test-secret-token');
  const parsedBody = JSON.parse(capturedBody);
  assert.equal(typeof parsedBody.id, 'string');
  assert.equal(parsedBody.id, '1');
  assert.equal(parsedBody.method, 'daemon.ping');
  assert.equal(res.pong, true);
  assert.equal(res.pid, 777);
  transport.dispose();
});

test('HttpWsTransport binds fetch to globalThis to prevent WebView2 Window Illegal invocation', async () => {
  const originalFetch = globalThis.fetch;
  function strictWindowFetch(_url, _init) {
    if (this !== globalThis) {
      throw new TypeError("Failed to execute 'fetch' on 'Window': Illegal invocation");
    }
    return Promise.resolve(
      new Response(
        JSON.stringify({ jsonrpc: '2.0', id: 1, result: { pong: true, pid: 1, uptimeMs: 1 } }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    );
  }
  globalThis.fetch = strictWindowFetch;
  try {
    const defaultTransport = new HttpWsTransport('http://127.0.0.1:7681', 'tok');
    const r1 = await defaultTransport.invoke(STRATA_METHODS.DAEMON_PING);
    assert.equal(r1.pong, true);
    defaultTransport.dispose();

    const explicitTransport = new HttpWsTransport(
      'http://127.0.0.1:7681',
      'tok',
      strictWindowFetch,
    );
    const r2 = await explicitTransport.invoke(STRATA_METHODS.DAEMON_PING);
    assert.equal(r2.pong, true);
    explicitTransport.dispose();
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('HttpWsTransport surfaces JSON-RPC domain errors as StrataRpcError', async () => {
  const fakeFetch = async () =>
    new Response(
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        error: { code: -32028, message: 'service not found' },
      }),
      { status: 200 },
    );

  const transport = new HttpWsTransport('http://127.0.0.1:7681', 'tok', fakeFetch);
  await assert.rejects(
    () => transport.invoke(STRATA_METHODS.SERVICE_START, { container: 'main', serviceId: 'none' }),
    (err) => err instanceof StrataRpcError && err.code === -32028,
  );
  transport.dispose();
});

test('MockStrataTransport supports managed services lifecycle and live state events', async () => {
  const transport = new MockStrataTransport('mock-tablet');
  const events = [];
  const unsub = transport.subscribe((ev) => events.push(ev));

  const initial = await transport.invoke(STRATA_METHODS.SERVICE_LIST, { container: 'main' });
  assert.ok(initial.length >= 2);

  const created = await transport.invoke(STRATA_METHODS.SERVICE_UPSERT, {
    container: 'main',
    spec: {
      id: 'caddy',
      name: 'Caddy 反向代理',
      command: ['caddy', 'run'],
      workdir: '/etc/caddy',
      env: { CADDY_ADMIN: 'off' },
      autoStart: true,
      restartPolicy: 'on-failure',
      maxRestarts: 5,
      backoffMs: 1500,
    },
  });
  assert.equal(created.id, 'caddy');
  assert.equal(created.status, 'stopped');

  const started = await transport.invoke(STRATA_METHODS.SERVICE_START, {
    container: 'main',
    serviceId: 'caddy',
  });
  assert.equal(started.status, 'running');
  assert.ok(events.some((e) => e.event === STRATA_EVENTS.SERVICE_STATE_CHANGED));

  const logs = await transport.invoke(STRATA_METHODS.SERVICE_LOGS, {
    container: 'main',
    serviceId: 'caddy',
  });
  assert.ok(logs.lines.length >= 1);

  const deleted = await transport.invoke(STRATA_METHODS.SERVICE_DELETE, {
    container: 'main',
    serviceId: 'caddy',
  });
  assert.equal(deleted.deleted, true);

  unsub();
  transport.dispose();
});

test('MockStrataTransport interactive PTY emits prompt and executes preview commands', async () => {
  const transport = new MockStrataTransport('mock-tablet');
  let output = '';
  let sessionId = '';

  const pty = transport.openPty('main', 100, 30, {
    onMeta: (m) => {
      sessionId = m.sessionId;
    },
    onData: (chunk) => {
      output += new TextDecoder().decode(chunk);
    },
  });

  await new Promise((r) => setTimeout(r, 60));
  assert.ok(sessionId.startsWith('mock-pty-main'));
  pty.sendInput('uname -a\r');
  await new Promise((r) => setTimeout(r, 30));
  assert.match(output, /aarch64 GNU\/Linux/);

  pty.close();
  transport.dispose();
});
