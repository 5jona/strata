import { type BridgeClient, shellQuote } from '@gojona/bridge';
import { STRATA_PROTOCOL, type StrataRpcMethodMap } from '../types.ts';

/** Canonical path to the Strata CLI installed inside the KernelSU module directory. */
export const STRATA_CLI_BIN = `/data/adb/modules/${STRATA_PROTOCOL.MODULE_ID}/bin/strata`;

/** Canonical path to the KernelSU service supervisor script. */
export const STRATA_SERVICE_SH = `/data/adb/modules/${STRATA_PROTOCOL.MODULE_ID}/service.sh`;

export type StrataKsuMethod = keyof StrataRpcMethodMap;

/** Result returned by `strata --json web token`. */
export interface KsuWebTokenResult {
  token: string;
  tokenPath: string;
}

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

/** Builds a non-blocking shell command launching `service.sh --now` in the background. */
export function buildStartDaemonShellCommand(): string {
  return `nohup /system/bin/sh ${shellQuote(STRATA_SERVICE_SH)} --now >/dev/null 2>&1 </dev/null & /system/bin/sleep 1`;
}

/** Invokes a typed `stratad` JSON-RPC method through `@gojona/bridge`'s KernelSU `exec` transport. */
export async function invokeStrataKsuRpc<M extends StrataKsuMethod>(
  bridge: Pick<BridgeClient, 'exec'>,
  method: M,
  params?: StrataRpcMethodMap[M]['params'],
  timeoutMs = 15_000,
): Promise<StrataRpcMethodMap[M]['result']> {
  const cmd = buildStrataRpcShellCommand(method, params);
  const res = await bridge.exec(cmd, { timeoutMs });
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
): Promise<KsuWebTokenResult> {
  const res = await bridge.exec(buildStrataReadTokenCommand(), { timeoutMs: 8_000 });
  if (res.errno !== 0) {
    const detail = res.stderr.trim() || res.stdout.trim() || `errno ${res.errno}`;
    throw new Error(`读取 Token 失败：${detail}`);
  }
  return JSON.parse(res.stdout.trim()) as KsuWebTokenResult;
}

/** Launches the `stratad` supervisor via KernelSU `service.sh --now`. */
export async function startStrataDaemonViaKsu(bridge: Pick<BridgeClient, 'exec'>): Promise<void> {
  const res = await bridge.exec(buildStartDaemonShellCommand(), { timeoutMs: 10_000 });
  if (res.errno !== 0) {
    const detail = res.stderr.trim() || res.stdout.trim() || `errno ${res.errno}`;
    throw new Error(`拉起守护进程失败：${detail}`);
  }
}
