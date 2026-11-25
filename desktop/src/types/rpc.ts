/**
 * Single-source-of-truth Strata JSON-RPC 2.0 method and event contracts re-exported
 * from `webui/src/generated/rpc.ts`.
 */

import {
  STRATA_EVENTS,
  STRATA_METHODS,
  type StrataEventPayloadMap,
  type StrataRpcMethodMap,
} from '../../../webui/src/generated/rpc.ts';

export { STRATA_EVENTS, STRATA_METHODS, type StrataEventPayloadMap, type StrataRpcMethodMap };

export type StrataDesktopRpcMap = StrataRpcMethodMap;
export type StrataDesktopMethod = keyof StrataRpcMethodMap;

// biome-ignore lint/suspicious/noConfusingVoidType: generated StrataRpcMethodMap uses void for parameterless methods
export type RpcInvokeArgs<P> = [P] extends [void] ? [] : [params: P];

export type StrataDesktopEventMap = StrataEventPayloadMap;
export type StrataDesktopEventName = keyof StrataEventPayloadMap;

export type StrataEventEnvelope = {
  [K in StrataDesktopEventName]: {
    event: K;
    payload: StrataDesktopEventMap[K];
  };
}[StrataDesktopEventName];

export class StrataRpcError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(message);
    this.name = 'StrataRpcError';
    this.code = code;
  }
}
