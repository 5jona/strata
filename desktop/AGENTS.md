# Strata Desktop Guidelines

Applies to `desktop/` (React 19 + TypeScript + Vite + xterm.js frontend, `crates/strata-desktop-core`, and `src-tauri` Tauri v2 shell).

- **Package & Dependency Boundaries**: Reuse `@gojona/ui` and `@gojona/config` for themes and components. Never depend on `@gojona/sideveil-sdk` or access `window.__POCHITA_HOST__` / `window.ksu` (enforced by `desktop/test/standards.test.mjs`).
- **Tauri v2 & WebView2 Invariants**: Keep `default = ["custom-protocol"]` in `src-tauri/Cargo.toml` so release builds embed `../dist` rather than querying `devUrl`. Always bind `fetch` to `globalThis` (detached `window.fetch` throws `Illegal invocation` in WebView2), send string JSON-RPC `id`s (bare numbers return `HTTP 422`), and keep CSP3 `connect-src` (`'self' http: ws:`) aligned with `stratad`'s `http://tauri.localhost` CORS allowlist.
- **WebSocket & Hook Stability**: Never place unstable inline parent callbacks (e.g., `onSessionBound`) or fresh per-render controller objects in WebSocket/PTY `useEffect` dependency arrays (prevents reconnect storms exhausting `stratad` PTY slots).
- **Rust Workspace Discipline**: Inherit `[workspace.lints]` in `desktop/Cargo.toml`, avoid `.unwrap()`/`.expect()` outside tests, and never invoke shell commands with unescaped user strings.
- **Size, Language & Privacy**: Keep source files `<= 300` lines (hard limit `400`). English for code/comments/docs; Simplified Chinese (`zh-CN`) for UI copy. Never include device serials, private IPs, or personal paths in code or tests.
- **Verification**: Run `mise run desktop:check` (or `pnpm check && cargo test` inside `desktop/`). When linking release `strata-desktop.exe` on Windows, terminate any running `strata-desktop.exe` instance first to avoid file-lock link errors.
