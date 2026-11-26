import { FitAddon } from '@xterm/addon-fit';
import { type ISearchResultChangeEvent, SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import {
  createTerminalKeyEventHandler,
  type TerminalKeybindingHandlers,
} from './terminalKeybindings.ts';
import { openExternalUrl, writeClipboardText } from './terminalPlatform.ts';
import { TERMINAL_FONT_FAMILY, type TerminalPrefs } from './terminalPrefs.ts';
import type { ResolvedTerminalTheme } from './terminalThemes.ts';

export interface HoverLinkState {
  url: string;
  x: number;
  y: number;
}

export interface UseXtermInstanceOptions {
  hostRef: RefObject<HTMLDivElement | null>;
  prefs: TerminalPrefs;
  resolvedTheme: ResolvedTerminalTheme;
  visible: boolean;
  focused: boolean;
  onData: (data: string) => void;
  onResize: (cols: number, rows: number) => void;
  onTitleChange?: (title: string) => void;
  onSearchResults: (ev: ISearchResultChangeEvent) => void;
  onReady: () => void;
  onDispose: () => void;
  getKeyHandlers: () => TerminalKeybindingHandlers;
}

const RESIZE_DEBOUNCE_MS = 80;

/**
 * Owns the xterm.js v6 Terminal instance, addons (Fit, Search, Unicode11, WebLinks, WebGL2),
 * debounced ResizeObserver, font-ready atlas refresh, Ctrl+Wheel zoom, and live prefs updates.
 */
export function useXtermInstance(options: UseXtermInstanceOptions) {
  const { hostRef, prefs, resolvedTheme, visible, focused } = options;
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  const optsRef = useRef(options);
  optsRef.current = options;

  const [rendererKind, setRendererKind] = useState<'webgl2' | 'dom'>('dom');
  const [dims, setDims] = useState({ cols: 80, rows: 24 });
  const [hoverLink, setHoverLink] = useState<HoverLinkState | null>(null);

  const disposeWebgl = useCallback(() => {
    const gl = webglRef.current;
    webglRef.current = null;
    if (!gl) return;
    try {
      gl.dispose();
    } catch {
      // Ignore already-disposed WebGL context
    }
  }, []);

  const safeFit = useCallback(() => {
    const host = hostRef.current;
    const fit = fitRef.current;
    const term = termRef.current;
    if (!host || !fit || !term || !host.offsetWidth || !host.offsetHeight) return;
    try {
      fit.fit();
      setDims((prev) =>
        prev.cols === term.cols && prev.rows === term.rows
          ? prev
          : { cols: term.cols, rows: term.rows },
      );
    } catch {
      // Ignore layout race during collapse
    }
  }, [hostRef]);

  const attachWebgl = useCallback(
    (term: Terminal, desired: 'webgl' | 'dom') => {
      disposeWebgl();
      if (desired === 'dom') {
        setRendererKind('dom');
        return;
      }
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => {
          webgl.dispose();
          if (webglRef.current === webgl) webglRef.current = null;
          setRendererKind('dom');
        });
        term.loadAddon(webgl);
        webglRef.current = webgl;
        setRendererKind('webgl2');
      } catch {
        setRendererKind('dom');
      }
    },
    [disposeWebgl],
  );

  // Mount xterm.js once per pane lifecycle
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const initialPrefs = optsRef.current.prefs;

    const term = new Terminal({
      allowProposedApi: true,
      cursorBlink: initialPrefs.cursorBlink,
      cursorStyle: initialPrefs.cursorStyle,
      fontSize: initialPrefs.fontSize,
      lineHeight: initialPrefs.lineHeight,
      fontFamily: TERMINAL_FONT_FAMILY,
      scrollback: 10000,
      minimumContrastRatio: 4.5,
      customGlyphs: true,
      rescaleOverlappingGlyphs: true,
      theme: optsRef.current.resolvedTheme.theme,
    });

    const fit = new FitAddon();
    const search = new SearchAddon();
    const unicode = new Unicode11Addon();
    const webLinks = new WebLinksAddon(
      (ev, uri) => {
        if (ev.ctrlKey || ev.metaKey) void openExternalUrl(uri);
      },
      {
        hover: (ev, uri) => setHoverLink({ url: uri, x: ev.clientX, y: ev.clientY }),
        leave: () => setHoverLink(null),
      },
    );

    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(unicode);
    term.loadAddon(webLinks);
    term.unicode.activeVersion = '11';
    term.open(host);

    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;
    attachWebgl(term, initialPrefs.renderer);

    term.attachCustomKeyEventHandler(
      createTerminalKeyEventHandler(term, () => optsRef.current.getKeyHandlers()),
    );

    const dataSub = term.onData((data) => optsRef.current.onData(data));
    const resizeSub = term.onResize(({ cols, rows }) => {
      optsRef.current.onResize(cols, rows);
      setDims({ cols, rows });
    });
    const titleSub = term.onTitleChange((title) => optsRef.current.onTitleChange?.(title));
    const selectionSub = term.onSelectionChange(() => {
      if (optsRef.current.prefs.copyOnSelect && term.hasSelection()) {
        const text = term.getSelection();
        if (text) void writeClipboardText(text);
      }
    });
    const searchSub = search.onDidChangeResults((ev) => optsRef.current.onSearchResults(ev));

    const handleWheel = (ev: WheelEvent) => {
      if (ev.ctrlKey || ev.metaKey) {
        ev.preventDefault();
        optsRef.current.getKeyHandlers().onStepFontSize(ev.deltaY < 0 ? 1 : -1);
      }
    };
    host.addEventListener('wheel', handleWheel, { passive: false });

    // rAF + 80ms debounced ResizeObserver eliminates resize flicker & SIGWINCH storms
    let rafId = 0;
    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    const scheduleFit = () => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => {
        debounceTimer = null;
        if (rafId) cancelAnimationFrame(rafId);
        rafId = requestAnimationFrame(() => {
          rafId = 0;
          safeFit();
        });
      }, RESIZE_DEBOUNCE_MS);
    };

    const ro = new ResizeObserver(() => {
      if (host.offsetWidth && host.offsetHeight) scheduleFit();
    });
    ro.observe(host);

    // Re-measure glyphs after bundled Maple Mono NF CN finishes loading
    let cancelled = false;
    void document.fonts?.ready.then(() => {
      if (!cancelled && termRef.current === term) {
        term.clearTextureAtlas();
        safeFit();
      }
    });

    safeFit();
    optsRef.current.onReady();

    return () => {
      cancelled = true;
      if (debounceTimer) clearTimeout(debounceTimer);
      if (rafId) cancelAnimationFrame(rafId);
      ro.disconnect();
      host.removeEventListener('wheel', handleWheel);
      dataSub.dispose();
      resizeSub.dispose();
      titleSub.dispose();
      selectionSub.dispose();
      searchSub.dispose();
      optsRef.current.onDispose();
      disposeWebgl();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
  }, [attachWebgl, disposeWebgl, hostRef, safeFit]);

  // Apply live theme, typography, and cursor updates without recreating xterm
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = resolvedTheme.theme;
    term.options.fontSize = prefs.fontSize;
    term.options.lineHeight = prefs.lineHeight;
    term.options.cursorStyle = prefs.cursorStyle;
    term.options.cursorBlink = prefs.cursorBlink;
    term.clearTextureAtlas();
    safeFit();
  }, [
    prefs.cursorBlink,
    prefs.cursorStyle,
    prefs.fontSize,
    prefs.lineHeight,
    resolvedTheme,
    safeFit,
  ]);

  // Toggle WebGL2 <-> DOM renderer cleanly on user preference change
  useEffect(() => {
    const term = termRef.current;
    if (term) attachWebgl(term, prefs.renderer);
  }, [attachWebgl, prefs.renderer]);

  // Re-fit and focus on tab/pane activation
  useEffect(() => {
    if (!visible) return;
    const raf = requestAnimationFrame(() => {
      safeFit();
      if (focused) termRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [focused, safeFit, visible]);

  return { termRef, searchRef, rendererKind, dims, hoverLink, safeFit };
}
