import {
  createTypedPluginClient,
  type RpcInvokeArgs,
  type TypedPluginBridge,
  type TypedPluginClient,
  type TypedPluginClientOptions,
  usePluginEvent,
  useTypedPluginClient,
} from '@pochita/sideveil-sdk';
import type { StrataEventPayloadMap, StrataRpcMethodMap } from '../types.ts';

/** Union of all Strata JSON-RPC method names supported by the SideVeil plugin bridge. */
export type StrataRpcMethod = keyof StrataRpcMethodMap;

/** Union of all Strata domain event names pushed over the SideVeil plugin bridge. */
export type StrataEventName = keyof StrataEventPayloadMap;

/** Compile-time-typed Strata plugin RPC and event client backed by `@pochita/sideveil-sdk`. */
export type StrataRpcClient = TypedPluginClient<StrataRpcMethodMap, StrataEventPayloadMap>;

/** Minimal plugin RPC invoker interface compatible with SideVeil bridge instances. */
export type StrataPluginInvoker = Pick<TypedPluginBridge, 'invokePlugin'> &
  Partial<Pick<TypedPluginBridge, 'onPluginEvent'>>;

const clientCache = new WeakMap<StrataPluginInvoker, StrataRpcClient>();
const noopUnsubscribe = (): void => {};

/** Creates a compile-time-typed Strata plugin client over a SideVeil `BridgeClient`. */
export function createStrataClient(
  bridge: TypedPluginBridge,
  options?: TypedPluginClientOptions<StrataRpcMethodMap>,
): StrataRpcClient {
  return createTypedPluginClient<StrataRpcMethodMap, StrataEventPayloadMap>(bridge, options);
}

/** React hook returning a memoized, zero-re-render typed Strata plugin client. */
export function useStrataClient(
  options?: TypedPluginClientOptions<StrataRpcMethodMap>,
): StrataRpcClient {
  return useTypedPluginClient<StrataRpcMethodMap, StrataEventPayloadMap>(options);
}

function getCachedClient(bridge: StrataPluginInvoker): StrataRpcClient {
  const existing = clientCache.get(bridge);
  if (existing) return existing;
  const typedBridge: TypedPluginBridge = {
    invokePlugin: bridge.invokePlugin.bind(bridge),
    onPluginEvent: bridge.onPluginEvent ? bridge.onPluginEvent.bind(bridge) : () => noopUnsubscribe,
  };
  const created = createStrataClient(typedBridge);
  clientCache.set(bridge, created);
  return created;
}

/** Invokes a Strata JSON-RPC method with compile-time checked params and result types. */
export function invokeStrataRpc<M extends StrataRpcMethod>(
  bridge: StrataPluginInvoker,
  method: M,
  ...args: RpcInvokeArgs<StrataRpcMethodMap[M]['params']>
): Promise<StrataRpcMethodMap[M]['result']> {
  return getCachedClient(bridge).invoke(method, ...args);
}

/** Subscribes to a Strata domain event with compile-time checked payload type. */
export function useStrataEvent<E extends StrataEventName>(
  event: E,
  handler: (payload: StrataEventPayloadMap[E]) => void,
): void {
  usePluginEvent<StrataEventPayloadMap[E]>(event, handler);
}
