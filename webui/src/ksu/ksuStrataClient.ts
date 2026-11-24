import { type BridgeClient, MockTransport, shellQuote } from '@gojona/bridge';
import { handleMockPluginInvoke } from '../lib/mock/handlers.ts';
import { createInitialMockState } from '../lib/mock/state.ts';
import type { StrataRpcMethodMap } from '../types.ts';

/** Canonical path to the Strata CLI installed inside the KernelSU module directory. */
export const STRATA_CLI_BIN = '/data/adb/modules/strata/bin/strata';

export type StrataKsuMethod = keyof StrataRpcMethodMap;

/** Builds a POSIX-safe shell command invoking `strata rpc <method> [json]`. */
export function buildStrataRpcShellCommand(method: string, params?: unknown): string {
  const parts = [shellQuote(STRATA_CLI_BIN), 'rpc', shellQuote(method)];
  if (params !== undefined) {
    parts.push(shellQuote(JSON.stringify(params)));
  }
  return `${parts.join(' ')} </dev/null`;
}

/** Builds a POSIX-safe shell command reading the active Bearer token via `strata --json web token`. */
export function buildStrataReadTokenCommand(): string {
  return `${shellQuote(STRATA_CLI_BIN)} --json web token </dev/null`;
}

/** Invokes a typed `stratad` JSON-RPC method through `@gojona/bridge`'s KernelSU `exec` transport. */
export async function invokeStrataKsuRpc<M extends StrataKsuMethod>(
  bridge: Pick<BridgeClient, 'exec'>,
  method: M,
  params?: StrataRpcMethodMap[M]['params'],
): Promise<StrataRpcMethodMap[M]['result']> {
  const cmd = buildStrataRpcShellCommand(method, params);
  const res = await bridge.exec(cmd, { timeoutMs: 15_000 });
  if (res.errno !== 0) {
    const detail = res.stderr.trim() || res.stdout.trim() || `errno ${res.errno}`;
    throw new Error(`strata rpc ${method} 失败：${detail}`);
  }
  const stdout = res.stdout.trim();
  if (!stdout) {
    throw new Error(`strata rpc ${method} 返回空响应`);
  }
  return JSON.parse(stdout) as StrataRpcMethodMap[M]['result'];
}

/** Reads the current `stratad` Bearer token via `strata --json web token` (root KSU context). */
export async function readStrataKsuWebToken(
  bridge: Pick<BridgeClient, 'exec'>,
): Promise<{ token: string; tokenPath: string }> {
  const res = await bridge.exec(buildStrataReadTokenCommand(), { timeoutMs: 8_000 });
  if (res.errno !== 0) {
    const detail = res.stderr.trim() || res.stdout.trim() || `errno ${res.errno}`;
    throw new Error(`读取 Token 失败：${detail}`);
  }
  return JSON.parse(res.stdout.trim()) as { token: string; tokenPath: string };
}

function unquoteSingle(token: string): string {
  if (token.startsWith("'") && token.endsWith("'")) {
    return token.slice(1, -1).replaceAll("'\\''", "'");
  }
  return token;
}

/**
 * Creates a standalone preview `MockTransport` handling `strata rpc` and `strata --json web token`
 * when opening the KSU WebUI in a desktop browser outside KernelSU Manager.
 */
export function createMockKsuStrataTransport(): MockTransport {
  const state = createInitialMockState();
  return new MockTransport({
    namespace: 'strata-ksu-preview',
    onExec: (cmd) => {
      if (cmd.includes('web token')) {
        return {
          errno: 0,
          stdout: JSON.stringify({ token: state.webToken, tokenPath: state.webStatus.tokenPath }),
          stderr: '',
        };
      }
      const normalizedCmd = cmd.replace(/\s+<\/dev\/null\s*$/, '');
      const rpcMatch = normalizedCmd.match(/\brpc\s+('[^']*'|\S+)(?:\s+(.+))?$/);
      if (!rpcMatch) {
        return { errno: 1, stdout: '', stderr: `unsupported mock command: ${cmd}` };
      }
      const method = unquoteSingle(rpcMatch[1] ?? '');
      const rawJson = rpcMatch[2] ? unquoteSingle(rpcMatch[2].trim()) : undefined;
      const params = rawJson ? (JSON.parse(rawJson) as unknown) : undefined;
      const result = handleMockPluginInvoke(state, () => {}, method, params);
      return { errno: 0, stdout: JSON.stringify(result), stderr: '' };
    },
  });
}
