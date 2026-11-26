import { detectRuntime, type MockTransportLike } from '@gojona/bridge';
import { useEffect, useMemo, useState } from 'react';

/** Dynamically loads a mock transport only in browser/mock preview runtimes. */
export function useLazyMockTransport(
  loader: () => Promise<MockTransportLike>,
  allowUrlOverride?: boolean,
) {
  const needsMock = useMemo(() => {
    const env = detectRuntime(allowUrlOverride === undefined ? undefined : { allowUrlOverride });
    return env === 'mock' || env === 'browser';
  }, [allowUrlOverride]);

  const [mockTransport, setMockTransport] = useState<MockTransportLike | null>(null);

  useEffect(() => {
    if (!needsMock) return;
    let active = true;
    void loader().then((transport) => {
      if (active) setMockTransport(transport);
    });
    return () => {
      active = false;
    };
  }, [loader, needsMock]);

  return { ready: !needsMock || mockTransport !== null, mockTransport };
}
