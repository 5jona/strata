import {
  useClipboard,
  useSyncedPluginTheme as useSdkSyncedPluginTheme,
} from '@gojona/sideveil-sdk';
import {
  type ColorMode,
  isValidThemeId,
  type ResolvedColorScheme,
  type ThemeId,
  useTheme,
} from '@gojona/ui';

/** Synchronized theme state and host clipboard helper returned by `useStrataPluginTheme`. */
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
 * Thin wrapper binding `@gojona/ui`'s `useTheme()` into `@gojona/sideveil-sdk`'s
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
