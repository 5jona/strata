import { definePluginManifest } from '@pochita/sideveil-sdk';

export const STRATA_PLUGIN_ID = 'strata.manager';

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
