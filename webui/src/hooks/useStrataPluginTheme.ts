import {
  useClipboard,
  useSyncedPluginTheme as useSdkSyncedPluginTheme,
} from '@pochita/sideveil-sdk';
import {
  type ColorMode,
  isValidThemeId,
  type ResolvedColorScheme,
  type ThemeId,
  useTheme,
} from '@pochita/ui';

export interface UseStrataPluginThemeResult {
  hydrated: boolean;
  isNativeHost: boolean;
  themeId: ThemeId;
  colorMode: ColorMode;
  resolvedScheme: ResolvedColorScheme;
  isDark: boolean;
  copyText: (text: string) => Promise<boolean>;
}

/**
 * Thin wrapper binding `@pochita/ui`'s `useTheme()` into `@pochita/sideveil-sdk`'s
 * `useSyncedPluginTheme()` and host-backed `useClipboard()`.
 */
export function useStrataPluginTheme(): UseStrataPluginThemeResult {
  const theme = useTheme();
  const synced = useSdkSyncedPluginTheme<ThemeId>({
    theme,
    isValidThemeId,
  });
  const clipboard = useClipboard();

  return {
    hydrated: synced.hydrated,
    isNativeHost: synced.isNativeHost,
    themeId: theme.themeId,
    colorMode: theme.colorMode,
    resolvedScheme: theme.resolvedScheme,
    isDark: synced.isDark,
    copyText: clipboard.writeText,
  };
}
