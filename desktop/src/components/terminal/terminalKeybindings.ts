import type { Terminal } from '@xterm/xterm';
import { isMacPlatform, readClipboardText, writeClipboardText } from './terminalPlatform.ts';

export interface TerminalKeybindingHandlers {
  onToggleSearch: () => void;
  onStepFontSize: (delta: number) => void;
  onResetFontSize: () => void;
  onClearTerminal: () => void;
  onNewTabShortcut?: () => void;
  onClosePaneShortcut?: () => void;
  onSplitRightShortcut?: () => void;
  onSplitDownShortcut?: () => void;
}

/**
 * Builds an xterm.js custom key event handler with CJK IME safety,
 * smart Ctrl+C / Ctrl+Shift+C/V / Cmd+C/V bracketed paste, zoom, and workspace shortcuts.
 */
export function createTerminalKeyEventHandler(
  term: Terminal,
  getHandlers: () => TerminalKeybindingHandlers,
): (ev: KeyboardEvent) => boolean {
  const mac = isMacPlatform();

  return (ev: KeyboardEvent): boolean => {
    // Never intercept IME composition events (Chinese / Japanese / Korean input)
    if (ev.isComposing || ev.keyCode === 229) {
      return true;
    }
    if (ev.type !== 'keydown') {
      return true;
    }

    const handlers = getHandlers();
    const primaryMod = mac ? ev.metaKey : ev.ctrlKey;

    // Search: Ctrl+Shift+F (all platforms) or Cmd+F (macOS)
    if (
      (ev.ctrlKey && ev.shiftKey && !ev.altKey && ev.code === 'KeyF') ||
      (mac && ev.metaKey && !ev.shiftKey && !ev.altKey && ev.code === 'KeyF')
    ) {
      ev.preventDefault();
      handlers.onToggleSearch();
      return false;
    }

    // Explicit copy: Ctrl+Shift+C (all platforms) or Cmd+C (macOS)
    if (
      (ev.ctrlKey && ev.shiftKey && !ev.altKey && ev.code === 'KeyC') ||
      (mac && ev.metaKey && !ev.shiftKey && !ev.altKey && ev.code === 'KeyC')
    ) {
      ev.preventDefault();
      const selection = term.getSelection();
      if (selection) {
        void writeClipboardText(selection);
      }
      return false;
    }

    // Smart Ctrl+C: copy when text is selected, otherwise pass SIGINT (^C) to PTY
    if (!mac && ev.ctrlKey && !ev.shiftKey && !ev.altKey && !ev.metaKey && ev.code === 'KeyC') {
      if (term.hasSelection()) {
        ev.preventDefault();
        const selection = term.getSelection();
        if (selection) {
          void writeClipboardText(selection);
        }
        term.clearSelection();
        return false;
      }
      return true;
    }

    // Paste using xterm.paste() so bracketed paste mode is honored safely
    if (
      (ev.ctrlKey && ev.shiftKey && !ev.altKey && ev.code === 'KeyV') ||
      (mac && ev.metaKey && !ev.shiftKey && !ev.altKey && ev.code === 'KeyV')
    ) {
      ev.preventDefault();
      void readClipboardText().then((txt) => {
        if (txt) term.paste(txt);
      });
      return false;
    }

    // Clear terminal: Cmd+K (macOS)
    if (mac && ev.metaKey && !ev.shiftKey && !ev.altKey && ev.code === 'KeyK') {
      ev.preventDefault();
      handlers.onClearTerminal();
      return false;
    }

    // Font zoom: Ctrl/Cmd + =, -, 0
    if (primaryMod && !ev.altKey && !ev.shiftKey) {
      if (ev.code === 'Equal' || ev.code === 'NumpadAdd' || ev.key === '+') {
        ev.preventDefault();
        handlers.onStepFontSize(1);
        return false;
      }
      if (ev.code === 'Minus' || ev.code === 'NumpadSubtract' || ev.key === '-') {
        ev.preventDefault();
        handlers.onStepFontSize(-1);
        return false;
      }
      if (ev.code === 'Digit0' || ev.code === 'Numpad0' || ev.key === '0') {
        ev.preventDefault();
        handlers.onResetFontSize();
        return false;
      }
    }

    // Workspace tab & split shortcuts
    if (ev.ctrlKey && ev.shiftKey && !ev.altKey && !ev.metaKey) {
      if (ev.code === 'KeyT' && handlers.onNewTabShortcut) {
        ev.preventDefault();
        handlers.onNewTabShortcut();
        return false;
      }
      if (ev.code === 'KeyW' && handlers.onClosePaneShortcut) {
        ev.preventDefault();
        handlers.onClosePaneShortcut();
        return false;
      }
      if (ev.code === 'KeyD' && handlers.onSplitRightShortcut) {
        ev.preventDefault();
        handlers.onSplitRightShortcut();
        return false;
      }
      if (ev.code === 'KeyE' && handlers.onSplitDownShortcut) {
        ev.preventDefault();
        handlers.onSplitDownShortcut();
        return false;
      }
    }

    return true;
  };
}
