import { handleContainerMockInvoke } from './containerHandlers.ts';
import { handleDaemonMockInvoke, type MockEmitPluginFn, UNHANDLED } from './daemonHandlers.ts';
import { type MockStrataState, toParamsRecord } from './state.ts';

export type { MockEmitPluginFn } from './daemonHandlers.ts';

/** Dispatches a single Strata plugin RPC method against the mutable mock state. */
export function handleMockPluginInvoke(
  state: MockStrataState,
  emitPlugin: MockEmitPluginFn,
  method: string,
  rawParams: unknown,
): unknown {
  const params = toParamsRecord(rawParams);
  const daemonResult = handleDaemonMockInvoke(state, emitPlugin, method, params);
  if (daemonResult !== UNHANDLED) {
    return daemonResult;
  }
  return handleContainerMockInvoke(state, emitPlugin, method, params);
}
