import type { InvokeOptions } from '@pochita/bridge';
import { usePluginEvent } from '@pochita/sideveil-sdk';
import type { StrataEventPayloadMap, StrataRpcMethodMap } from '../types.ts';

export type StrataRpcMethod = keyof StrataRpcMethodMap;
export type StrataEventName = keyof StrataEventPayloadMap;

type RpcParams<M extends StrataRpcMethod> = StrataRpcMethodMap[M]['params'];
type RpcResult<M extends StrataRpcMethod> = StrataRpcMethodMap[M]['result'];

export interface StrataPluginInvoker {
  invokePlugin: <T = unknown>(
    method: string,
    params?: unknown,
    options?: InvokeOptions,
  ) => Promise<T>;
}

/** Invokes a Strata JSON-RPC method with compile-time checked params and result types. */
export function invokeStrataRpc<M extends StrataRpcMethod>(
  bridge: StrataPluginInvoker,
  method: M,
  ...args: [undefined] extends [RpcParams<M>]
    ? [params?: undefined, options?: InvokeOptions]
    : [params: RpcParams<M>, options?: InvokeOptions]
): Promise<RpcResult<M>> {
  const [params, options] = args;
  return bridge.invokePlugin<RpcResult<M>>(method, params, options);
}

/** Subscribes to a Strata domain event with compile-time checked payload type. */
export function useStrataEvent<E extends StrataEventName>(
  event: E,
  handler: (payload: StrataEventPayloadMap[E]) => void,
): void {
  usePluginEvent<StrataEventPayloadMap[E]>(event, handler);
}
