import { type BridgeOptions, BridgeProvider } from '@gojona/bridge';
import { ErrorBoundary, ThemeProvider, ToastProvider } from '@gojona/ui';
import { useMemo } from 'react';
import { useLazyMockTransport } from '../shared/useLazyMockTransport.ts';
import { KsuTabsWorkspace } from './KsuTabsWorkspace.tsx';
import { useKsuStrataRuntime } from './useKsuStrataRuntime.ts';

function KsuStrataWorkbench() {
  const rt = useKsuStrataRuntime();
  const daemonOnline = rt.daemon !== null && !rt.lastError;
  return <KsuTabsWorkspace active={rt.activeContainer} daemonOnline={daemonOnline} rt={rt} />;
}

const loadKsuMock = () =>
  import('./mockKsuTransport.ts').then((m) => m.createMockKsuStrataTransport());

/** Root standalone KernelSU WebUI component (`strata/webroot/`). */
export function KsuStrataApp() {
  const { ready, mockTransport } = useLazyMockTransport(loadKsuMock);

  const bridgeOptions = useMemo<BridgeOptions>(
    () => ({
      mockNamespace: 'strata-ksu-webui',
      ...(mockTransport ? { mockTransport } : {}),
    }),
    [mockTransport],
  );

  if (!ready) return null;

  return (
    <ErrorBoundary>
      <BridgeProvider options={bridgeOptions}>
        <ThemeProvider>
          <ToastProvider>
            <KsuStrataWorkbench />
          </ToastProvider>
        </ThemeProvider>
      </BridgeProvider>
    </ErrorBoundary>
  );
}
