import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { open as openShellUrl } from '@tauri-apps/plugin-shell';
import { isTauriRuntime } from '../../transport/tauriBridge.ts';

/** Returns `true` on macOS for Command-key shortcut adaptation. */
export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent);
}

/** Reads plain text from the native Tauri clipboard with Web Clipboard fallback. */
export async function readClipboardText(): Promise<string> {
  if (isTauriRuntime()) {
    try {
      const text = await readText();
      return text ?? '';
    } catch {
      // Fall through to browser clipboard API if native read fails
    }
  }
  try {
    if (typeof navigator !== 'undefined' && navigator.clipboard?.readText) {
      return await navigator.clipboard.readText();
    }
  } catch {
    // Ignore permission errors in headless/unfocused browser preview
  }
  return '';
}

/** Writes plain text to the native Tauri clipboard with Web Clipboard fallback. */
export async function writeClipboardText(text: string): Promise<boolean> {
  if (!text) return false;
  if (isTauriRuntime()) {
    try {
      await writeText(text);
      return true;
    } catch {
      // Fall through to browser clipboard API
    }
  }
  try {
    if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Ignore clipboard rejection
  }
  return false;
}

/** Opens an `http:` or `https:` URL in the system browser via Tauri shell plugin. */
export async function openExternalUrl(rawUrl: string): Promise<void> {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return;

  if (isTauriRuntime()) {
    try {
      await openShellUrl(parsed.toString());
      return;
    } catch {
      // Fall back to window.open below
    }
  }
  if (typeof window !== 'undefined') {
    window.open(parsed.toString(), '_blank', 'noopener,noreferrer');
  }
}
