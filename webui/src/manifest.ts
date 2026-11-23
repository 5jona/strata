import { definePluginManifest } from '@pochita/sideveil-sdk';
import { STRATA_PROTOCOL } from './types.ts';

/** Canonical SideVeil plugin identifier for the Strata container manager UI. */
export const STRATA_PLUGIN_ID = STRATA_PROTOCOL.SIDEVEIL_PLUGIN_ID;

/** Static SideVeil plugin manifest synchronized with `plugins/strata.manager/manifest.json`. */
export const STRATA_MANIFEST = definePluginManifest({
  id: STRATA_PLUGIN_ID,
  name: 'Strata 容器',
  version: '0.1.0',
  icon: 'web/icon.svg',
  triggers: {
    packages: [],
    activities: [],
    global: true,
    priority: 30,
  },
  entry: 'web/index.html',
  backend: 'uds-native',
  capabilities: ['toast', 'clipboard', 'haptics', 'native-hud'],
  surface: {
    backdrop: 'native-glass',
    presentation: 'drawer',
    crossOriginIsolated: false,
    widthDp: 344,
    heightPercent: 68,
  },
});
