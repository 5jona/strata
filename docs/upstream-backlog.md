# Upstream Optimization Backlog (Phase 2 -> Phase 3 — Resolved)

This backlog records real, general-purpose gaps encountered while building `strata.manager` as SideVeil's first complex production `uds-native` plugin. Every item was implemented generically upstream in `sideveil` or `pochita-webui-kit` in Phase 3 with zero Strata-specific business logic, and subsequently adopted by `strata`.

## 1. `sideveil` (`hub/` & `android/host-app/`)

1. **[Resolved in Phase 3] Configurable `plugin.invoke` RPC Timeout**
   - Added bounded `timeoutMs` (`1_000..=60_000` ms, default `8_000` ms) to `PluginInvokeParams` in `sideveil-protocol`, honored in `sideveild/src/plugin_backends.rs`, `android/protocol/Protocol.kt` (`SideVeilTimeouts`), and `android/host-app/DaemonClient.kt`.
   - Adopted in `strata/webui/src/hooks/useStrataRuntime.ts` for container lifecycle (`20–25s`) and `compact`/`fsck` (`45s`) RPCs.

2. **[Resolved in Phase 3] CLI Backend Invocation for E2E Plugin Testing**
   - Extended `handle_plugin_invoke` (`sideveild/src/rpc/sandbox.rs`) to allow privileged `RpcCaller::Ctl` callers to pass an explicit `pluginId` (while keeping sandboxed WebViews strictly pinned to their server-stamped `pluginId`) and added `sideveil-ctl plugins invoke <plugin-id> <method> [json] [--timeout-ms <ms>]`.
   - Exercised live in both `sideveil/scripts/verify-live.py` and `strata/scripts/verify-live.py`.

3. **[Resolved in Phase 3] Atomic Plugin Hot-Install & Subdirectory `inotify` Watching**
   - Added atomic staged `plugins.install` RPC (`sideveild/src/plugin_registry/installer.rs`) + `sideveil-ctl plugins install <source-dir>` CLI command, rejecting symlinks and built-in overwrites, and extended `InotifyDirWatcher` (`sideveild/src/inotify.rs`) to watch immediate plugin subdirectories with debounced rescanning.
   - Adopted in `strata/scripts/sync-sideveil-plugin.sh` (preferring `sideveil-ctl plugins install` when `sideveild` is online, with offline atomic dot-staging fallback).

4. **[Resolved in Phase 3] Ephemeral Event Delivery Tier & Per-Plugin Rate Protection**
   - Added optional `ephemeral: bool` on `PluginBackendEvent` routed to `emit_plugin_scoped_event_ephemeral(...)` and enforced `PluginEventRateLimiter` (120 reliable events/s) emitting `plugin.eventLagged` (`{ dropped }`) on rollover (`sideveild/src/uds_gateway/backend_session.rs`).
   - Adopted in `strata/crates/stratad/src/sideveil_seam/bridge.rs` (marking `task.output` frames `ephemeral: true`) and `strata/webui/src/hooks/useStrataTasks.ts` (listening for `plugin.eventLagged`).

5. **[Resolved in Phase 3] On-Device & Container SSH Verification & Reload Harness**
   - Promoted SideVeil live verification to `sideveil/scripts/verify-live.py` supporting `--mode auto|ssh|host-exec|local` (auto-detecting `STRATA_SSH_HOST`/`SSH_TARGET` or Strata's on-device `host-exec` / Python 3 environment) and `--ssh <host|user@host>` container deployment in `sideveil/reload.sh`.

## 2. `pochita-webui-kit` (`@pochita/sideveil-sdk`, `@pochita/ui`, `@pochita/bridge`)

6. **[Resolved in Phase 3] Promoted `useSyncedPluginTheme()` into `@pochita/sideveil-sdk`**
   - Exported `useSyncedPluginTheme` and `extractPluginAppearance` from `@pochita/sideveil-sdk`.
   - Adopted by both `sideveil/webui/src/shared/useSyncedPluginTheme.ts` and `strata/webui/src/hooks/useStrataPluginTheme.ts`.

7. **[Resolved in Phase 3] Added `useClipboard()` to `@pochita/sideveil-sdk` & `LogConsole.onCopy` to `@pochita/ui`**
   - Exported `useClipboard()` (`readText` / `writeText` via host `clipboard.get` / `clipboard.set` with Web Clipboard fallback) from `@pochita/sideveil-sdk` and added optional `onCopy` prop to `<LogConsole>` in `@pochita/ui`.
   - Adopted in `sideveil/webui/src/panel/PanelApp.tsx` and `strata/webui/src/components/{TasksTab,ConfigTab}.tsx`.

8. **[Resolved in Phase 3] Per-Call `InvokeOptions { timeoutMs }` on `bridge.invokePlugin` & `usePluginRpc`**
   - Added `InvokeOptions` support end-to-end across `@pochita/bridge` (`invoke`, `invokePlugin`) and `@pochita/sideveil-sdk` (`usePluginRpc`).
   - Adopted by `strata/webui/src/hooks/useStrataRuntime.ts`.
