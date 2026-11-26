import type { ITheme } from '@xterm/xterm';

export type TerminalThemeId =
  | 'auto'
  | 'catppuccin-mocha'
  | 'catppuccin-latte'
  | 'tokyo-night'
  | 'one-dark';

export interface TerminalThemeOption {
  id: TerminalThemeId;
  label: string;
  description: string;
  swatches: [string, string, string];
}

export interface TerminalSearchColors {
  matchBackground: string;
  matchBorder: string;
  matchOverviewRuler: string;
  activeMatchBackground: string;
  activeMatchBorder: string;
  activeMatchColorOverviewRuler: string;
}

export interface ResolvedTerminalTheme {
  id: Exclude<TerminalThemeId, 'auto'>;
  isDark: boolean;
  theme: ITheme;
  background: string;
  searchColors: TerminalSearchColors;
}

export const TERMINAL_THEME_OPTIONS: TerminalThemeOption[] = [
  {
    id: 'auto',
    label: '跟随系统主题',
    description: '深色自动使用 Catppuccin Mocha，浅色使用 Latte',
    swatches: ['#1e1e2e', '#eff1f5', '#89b4fa'],
  },
  {
    id: 'catppuccin-mocha',
    label: 'Catppuccin Mocha',
    description: '柔和温暖的深色现代配色',
    swatches: ['#1e1e2e', '#cdd6f4', '#cba6f7'],
  },
  {
    id: 'catppuccin-latte',
    label: 'Catppuccin Latte',
    description: '高对比护眼浅色主题',
    swatches: ['#eff1f5', '#4c4f69', '#1e66f5'],
  },
  {
    id: 'tokyo-night',
    label: 'Tokyo Night',
    description: '霓虹蓝紫深邃夜色风格',
    swatches: ['#1a1b26', '#c0caf5', '#7aa2f7'],
  },
  {
    id: 'one-dark',
    label: 'One Dark',
    description: '经典 Atom / VS Code 编程深色',
    swatches: ['#282c34', '#abb2bf', '#61afef'],
  },
];

const CATPPUCCIN_MOCHA: ITheme = {
  background: '#1e1e2e',
  foreground: '#cdd6f4',
  cursor: '#f5e0dc',
  cursorAccent: '#1e1e2e',
  selectionBackground: 'rgba(137, 180, 250, 0.28)',
  selectionInactiveBackground: 'rgba(137, 180, 250, 0.14)',
  black: '#45475a',
  red: '#f38ba8',
  green: '#a6e3a1',
  yellow: '#f9e2af',
  blue: '#89b4fa',
  magenta: '#f5c2e7',
  cyan: '#94e2d5',
  white: '#bac2de',
  brightBlack: '#585b70',
  brightRed: '#f38ba8',
  brightGreen: '#a6e3a1',
  brightYellow: '#f9e2af',
  brightBlue: '#89b4fa',
  brightMagenta: '#f5c2e7',
  brightCyan: '#94e2d5',
  brightWhite: '#a6adc8',
};

const CATPPUCCIN_LATTE: ITheme = {
  background: '#eff1f5',
  foreground: '#4c4f69',
  cursor: '#dc8a78',
  cursorAccent: '#eff1f5',
  selectionBackground: 'rgba(30, 102, 245, 0.22)',
  selectionInactiveBackground: 'rgba(30, 102, 245, 0.12)',
  black: '#5c5f77',
  red: '#d20f39',
  green: '#40a02b',
  yellow: '#df8e1d',
  blue: '#1e66f5',
  magenta: '#ea76cb',
  cyan: '#179299',
  white: '#acb0be',
  brightBlack: '#6c6f85',
  brightRed: '#d20f39',
  brightGreen: '#40a02b',
  brightYellow: '#df8e1d',
  brightBlue: '#1e66f5',
  brightMagenta: '#ea76cb',
  brightCyan: '#179299',
  brightWhite: '#bcc0cc',
};

const TOKYO_NIGHT: ITheme = {
  background: '#1a1b26',
  foreground: '#c0caf5',
  cursor: '#c0caf5',
  cursorAccent: '#1a1b26',
  selectionBackground: 'rgba(122, 162, 247, 0.28)',
  selectionInactiveBackground: 'rgba(122, 162, 247, 0.14)',
  black: '#15161e',
  red: '#f7768e',
  green: '#9ece6a',
  yellow: '#e0af68',
  blue: '#7aa2f7',
  magenta: '#bb9af7',
  cyan: '#7dcfff',
  white: '#a9b1d6',
  brightBlack: '#414868',
  brightRed: '#f7768e',
  brightGreen: '#9ece6a',
  brightYellow: '#e0af68',
  brightBlue: '#7aa2f7',
  brightMagenta: '#bb9af7',
  brightCyan: '#7dcfff',
  brightWhite: '#c0caf5',
};

const ONE_DARK: ITheme = {
  background: '#282c34',
  foreground: '#abb2bf',
  cursor: '#528bff',
  cursorAccent: '#282c34',
  selectionBackground: 'rgba(97, 175, 239, 0.28)',
  selectionInactiveBackground: 'rgba(97, 175, 239, 0.14)',
  black: '#282c34',
  red: '#e06c75',
  green: '#98c379',
  yellow: '#e5c07b',
  blue: '#61afef',
  magenta: '#c678dd',
  cyan: '#56b6c2',
  white: '#abb2bf',
  brightBlack: '#5c6370',
  brightRed: '#e06c75',
  brightGreen: '#98c379',
  brightYellow: '#e5c07b',
  brightBlue: '#61afef',
  brightMagenta: '#c678dd',
  brightCyan: '#56b6c2',
  brightWhite: '#ffffff',
};

const DARK_SEARCH: TerminalSearchColors = {
  matchBackground: '#45475a',
  matchBorder: '#89b4fa',
  matchOverviewRuler: '#89b4fa',
  activeMatchBackground: '#f9e2af',
  activeMatchBorder: '#fab387',
  activeMatchColorOverviewRuler: '#f9e2af',
};

const LIGHT_SEARCH: TerminalSearchColors = {
  matchBackground: '#ccd0da',
  matchBorder: '#1e66f5',
  matchOverviewRuler: '#1e66f5',
  activeMatchBackground: '#df8e1d',
  activeMatchBorder: '#fe640b',
  activeMatchColorOverviewRuler: '#df8e1d',
};

const PALETTES: Record<Exclude<TerminalThemeId, 'auto'>, ResolvedTerminalTheme> = {
  'catppuccin-mocha': {
    id: 'catppuccin-mocha',
    isDark: true,
    theme: CATPPUCCIN_MOCHA,
    background: '#1e1e2e',
    searchColors: DARK_SEARCH,
  },
  'catppuccin-latte': {
    id: 'catppuccin-latte',
    isDark: false,
    theme: CATPPUCCIN_LATTE,
    background: '#eff1f5',
    searchColors: LIGHT_SEARCH,
  },
  'tokyo-night': {
    id: 'tokyo-night',
    isDark: true,
    theme: TOKYO_NIGHT,
    background: '#1a1b26',
    searchColors: DARK_SEARCH,
  },
  'one-dark': {
    id: 'one-dark',
    isDark: true,
    theme: ONE_DARK,
    background: '#282c34',
    searchColors: DARK_SEARCH,
  },
};

/** Resolves a theme preset (or `auto`) against the current desktop color scheme. */
export function resolveTerminalTheme(
  themeId: TerminalThemeId,
  resolvedScheme: 'light' | 'dark',
): ResolvedTerminalTheme {
  if (themeId === 'auto') {
    return resolvedScheme === 'dark' ? PALETTES['catppuccin-mocha'] : PALETTES['catppuccin-latte'];
  }
  return PALETTES[themeId] ?? PALETTES['catppuccin-mocha'];
}
