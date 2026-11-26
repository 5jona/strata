import {
  useClipboard,
  useSyncedPluginTheme as useSdkSyncedPluginTheme,
  useSideVeilPlugin,
} from '@gojona/sideveil-sdk';
import {
  type ColorMode,
  isValidThemeId,
  type ResolvedColorScheme,
  type ThemeId,
  useTheme,
} from '@gojona/ui';
import { useCallback } from 'react';
import { isSideveilPermissionError } from './permissionGuide.ts';

/** Result of copying text via SideVeil host clipboard with permission denial detection. */
export interface CopyTextResult {
  ok: boolean;
  permissionDenied: boolean;
}

/** Synchronized theme state and host clipboard helper returned by `useStrataPluginTheme`. */
export interface UseStrataPluginThemeResult {
  hydrated: boolean;
  isNativeHost: boolean;
  themeId: ThemeId;
  colorMode: ColorMode;
  resolvedScheme: ResolvedColorScheme;
  isDark: boolean;
  copyText: (text: string) => Promise<boolean>;
  copyTextDetailed: (text: string) => Promise<CopyTextResult>;
}

/**
 * Thin wrapper binding `@gojona/ui`'s `useTheme()` into `@gojona/sideveil-sdk`'s
 * `useSyncedPluginTheme()` and host-backed `useClipboard()`.
 */
export function useStrataPluginTheme(): UseStrataPluginThemeResult {
  const { bridge } = useSideVeilPlugin();
  const theme = useTheme();
  const synced = useSdkSyncedPluginTheme<ThemeId>({
    theme,
    isValidThemeId,
  });
  const clipboard = useClipboard();

  const copyTextDetailed = useCallback(
    async (text: string): Promise<CopyTextResult> => {
      if (bridge.env === 'host') {
        try {
          await bridge.invoke('clipboard.set', { text });
          return { ok: true, permissionDenied: false };
        } catch (err) {
          if (isSideveilPermissionError(err)) {
            return { ok: false, permissionDenied: true };
          }
        }
      }
      const ok = await clipboard.writeText(text);
      return { ok, permissionDenied: false };
    },
    [bridge, clipboard],
  );

  return {
    hydrated: synced.hydrated,
    isNativeHost: synced.isNativeHost,
    themeId: theme.themeId,
    colorMode: theme.colorMode,
    resolvedScheme: theme.resolvedScheme,
    isDark: synced.isDark,
    copyText: clipboard.writeText,
    copyTextDetailed,
  };
}
