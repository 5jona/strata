import { useCallback, useState } from 'react';
import type { TerminalThemeId } from './terminalThemes.ts';

export type TerminalShellId = 'auto' | 'bash' | 'zsh' | 'fish' | 'nu';
export type TerminalCursorStyle = 'block' | 'bar' | 'underline';
export type TerminalRendererPref = 'webgl' | 'dom';

export interface TerminalShellOption {
  id: TerminalShellId;
  label: string;
  shortLabel: string;
  hint: string;
}

export const TERMINAL_SHELL_OPTIONS: TerminalShellOption[] = [
  { id: 'auto', label: '自动', shortLabel: '自动', hint: '按容器默认 Shell 启动' },
  { id: 'bash', label: 'Bash', shortLabel: 'bash', hint: '通用标准 /bin/bash' },
  { id: 'zsh', label: 'Zsh', shortLabel: 'zsh', hint: '兼容 POSIX 的现代交互 Shell' },
  { id: 'fish', label: 'Fish', shortLabel: 'fish', hint: '开箱即用的智能补全 Shell' },
  { id: 'nu', label: 'Nushell', shortLabel: 'nu', hint: '结构化数据管道 Shell' },
];

export const TERMINAL_FONT_FAMILY =
  "'Maple Mono NF CN', 'JetBrains Mono', 'Cascadia Code', 'Noto Sans Mono CJK SC', 'PingFang SC', 'Microsoft YaHei', monospace";

export const MIN_FONT_SIZE = 11;
export const MAX_FONT_SIZE = 22;
export const DEFAULT_FONT_SIZE = 14;
export const MIN_LINE_HEIGHT = 1.0;
export const MAX_LINE_HEIGHT = 1.5;

export interface TerminalPrefs {
  themeId: TerminalThemeId;
  fontSize: number;
  lineHeight: number;
  cursorStyle: TerminalCursorStyle;
  cursorBlink: boolean;
  renderer: TerminalRendererPref;
  copyOnSelect: boolean;
  rightClickPaste: boolean;
  defaultShell: TerminalShellId;
}

export const DEFAULT_TERMINAL_PREFS: TerminalPrefs = {
  themeId: 'auto',
  fontSize: DEFAULT_FONT_SIZE,
  lineHeight: 1.2,
  cursorStyle: 'block',
  cursorBlink: true,
  renderer: 'webgl',
  copyOnSelect: false,
  rightClickPaste: false,
  defaultShell: 'auto',
};

const STORAGE_KEY = 'strata-desktop:terminal-prefs:v1';

const VALID_THEMES = new Set<TerminalThemeId>([
  'auto',
  'catppuccin-mocha',
  'catppuccin-latte',
  'tokyo-night',
  'one-dark',
]);
const VALID_CURSORS = new Set<TerminalCursorStyle>(['block', 'bar', 'underline']);
const VALID_SHELLS = new Set<TerminalShellId>(['auto', 'bash', 'zsh', 'fish', 'nu']);

export function clampFontSize(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_FONT_SIZE;
  return Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, Math.round(value)));
}

export function clampLineHeight(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_TERMINAL_PREFS.lineHeight;
  return Math.max(MIN_LINE_HEIGHT, Math.min(MAX_LINE_HEIGHT, Math.round(value * 10) / 10));
}

export function formatShellLabel(shell?: string): string {
  if (!shell || shell === 'auto') return '自动';
  const normalized = shell.split('/').pop()?.toLowerCase() ?? shell.toLowerCase();
  const matched = TERMINAL_SHELL_OPTIONS.find(
    (opt) => opt.id === normalized || opt.shortLabel === normalized,
  );
  return matched ? matched.label : normalized;
}

/** Loads persisted terminal preferences from `localStorage` with schema validation. */
export function loadTerminalPrefs(): TerminalPrefs {
  if (typeof window === 'undefined') return { ...DEFAULT_TERMINAL_PREFS };
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULT_TERMINAL_PREFS };
    const parsed = JSON.parse(raw) as Partial<TerminalPrefs>;
    return {
      themeId:
        parsed.themeId && VALID_THEMES.has(parsed.themeId)
          ? parsed.themeId
          : DEFAULT_TERMINAL_PREFS.themeId,
      fontSize:
        typeof parsed.fontSize === 'number'
          ? clampFontSize(parsed.fontSize)
          : DEFAULT_TERMINAL_PREFS.fontSize,
      lineHeight:
        typeof parsed.lineHeight === 'number'
          ? clampLineHeight(parsed.lineHeight)
          : DEFAULT_TERMINAL_PREFS.lineHeight,
      cursorStyle:
        parsed.cursorStyle && VALID_CURSORS.has(parsed.cursorStyle)
          ? parsed.cursorStyle
          : DEFAULT_TERMINAL_PREFS.cursorStyle,
      cursorBlink:
        typeof parsed.cursorBlink === 'boolean'
          ? parsed.cursorBlink
          : DEFAULT_TERMINAL_PREFS.cursorBlink,
      renderer:
        parsed.renderer === 'dom' || parsed.renderer === 'webgl'
          ? parsed.renderer
          : DEFAULT_TERMINAL_PREFS.renderer,
      copyOnSelect:
        typeof parsed.copyOnSelect === 'boolean'
          ? parsed.copyOnSelect
          : DEFAULT_TERMINAL_PREFS.copyOnSelect,
      rightClickPaste:
        typeof parsed.rightClickPaste === 'boolean'
          ? parsed.rightClickPaste
          : DEFAULT_TERMINAL_PREFS.rightClickPaste,
      defaultShell:
        parsed.defaultShell && VALID_SHELLS.has(parsed.defaultShell)
          ? parsed.defaultShell
          : DEFAULT_TERMINAL_PREFS.defaultShell,
    };
  } catch {
    return { ...DEFAULT_TERMINAL_PREFS };
  }
}

export function saveTerminalPrefs(prefs: TerminalPrefs): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // Ignore storage quota errors
  }
}

/** React hook holding persisted terminal appearance and interaction settings. */
export function useTerminalPrefs() {
  const [prefs, setPrefs] = useState<TerminalPrefs>(() => loadTerminalPrefs());

  const updatePrefs = useCallback((patch: Partial<TerminalPrefs>) => {
    setPrefs((prev) => {
      const next: TerminalPrefs = {
        ...prev,
        ...patch,
        fontSize: patch.fontSize !== undefined ? clampFontSize(patch.fontSize) : prev.fontSize,
        lineHeight:
          patch.lineHeight !== undefined ? clampLineHeight(patch.lineHeight) : prev.lineHeight,
      };
      saveTerminalPrefs(next);
      return next;
    });
  }, []);

  const stepFontSize = useCallback((delta: number) => {
    setPrefs((prev) => {
      const nextFontSize = clampFontSize(prev.fontSize + delta);
      if (nextFontSize === prev.fontSize) return prev;
      const next = { ...prev, fontSize: nextFontSize };
      saveTerminalPrefs(next);
      return next;
    });
  }, []);

  const resetFontSize = useCallback(() => {
    updatePrefs({ fontSize: DEFAULT_FONT_SIZE });
  }, [updatePrefs]);

  return { prefs, updatePrefs, stepFontSize, resetFontSize };
}
