import { definePluginManifest, type PluginManifest } from '@gojona/sideveil-sdk';
import { STRATA_PROTOCOL } from './types.ts';

/** Canonical SideVeil plugin identifier for the Strata container manager UI. */
export const STRATA_PLUGIN_ID = STRATA_PROTOCOL.SIDEVEIL_PLUGIN_ID;

/** SideVeil v1 plugin manifest extension carrying schema metadata fields. */
export interface StrataPluginManifestV1 extends PluginManifest {
  schemaVersion: number;
  apiVersion: number;
  minHostVersion: string;
  description: string;
  defaultEnabled: boolean;
}

const baseManifest = definePluginManifest({
  id: STRATA_PLUGIN_ID,
  name: 'Strata 容器',
  version: '0.2.0',
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

/** Static SideVeil v1 plugin manifest synchronized with `plugins/strata.manager/manifest.json`. */
export const STRATA_MANIFEST: StrataPluginManifestV1 = {
  schemaVersion: 1,
  apiVersion: 1,
  minHostVersion: '0.1.0',
  ...baseManifest,
  description: 'Android Linux 容器快捷控制、托管服务与 PC 配对侧边栏',
  defaultEnabled: true,
};
