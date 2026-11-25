import { MockTransport } from '@gojona/bridge';
import { STRATA_PLUGIN_ID } from '../manifest.ts';
import { handleMockPluginInvoke } from '../shared/mock/handlers.ts';
import { createInitialMockState } from '../shared/mock/state.ts';

export type { MockEmitPluginFn } from '../shared/mock/handlers.ts';
export { handleMockPluginInvoke } from '../shared/mock/handlers.ts';
export type { MockStrataState, MockTaskRecord } from '../shared/mock/state.ts';
export {
  applyMockConfigPatch,
  cloneConfig,
  cloneSnapshot,
  createInitialMockState,
} from '../shared/mock/state.ts';

/**
 * Creates a realistic stateful MockTransport for standalone browser preview and automated tests.
 */
export function createMockStrataTransport(): MockTransport {
  const state = createInitialMockState();

  const transport = new MockTransport({
    namespace: 'strata-manager-preview',
    onRpc: (method) =>
      method === 'config.get'
        ? { panel: { themeId: 'coloros-glass', colorMode: 'system' } }
        : { ok: true },
    onPluginInvoke: (method, rawParams) =>
      handleMockPluginInvoke(
        state,
        (event, payload) => {
          transport.emit('plugin.event', { pluginId: STRATA_PLUGIN_ID, event, payload });
        },
        method,
        rawParams,
      ),
  });

  return transport;
}
