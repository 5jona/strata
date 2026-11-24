# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-25

### Added

- **Cross-Platform Desktop Manager (`desktop/`) & Standalone KernelSU WebUI (`webroot/`)**:
  - Tauri v2 desktop companion (`Strata Desktop`) with USB ADB forwarding, LAN pairing, interactive xterm.js PTY shell, and host-side `strata-desktop-core` probe/discovery crate.
  - Standalone KernelSU WebUI (`webroot/`) and authenticated `stratad` HTTP/WebSocket control plane with CORS middleware and `http://tauri.localhost` origin allowlisting.
- **Version Parity Gate**: Added cross-stack version and `versionCode` (`MAJOR * 10000 + MINOR * 100 + PATCH`) alignment assertions in `webui/test/contracts.test.mjs`.

### Fixed

- Fixed `HttpWsTransport` detached `window.fetch` invocation (`Illegal invocation`) and numeric JSON-RPC `id` serialization (`HTTP 422`) in Strata Desktop, plus CSP3 `connect-src` permissions for WebView2.
- Fixed PTY WebSocket reconnect loop (`HTTP 429` / `1006`) by stabilizing `TerminalPane` callbacks, deduplicating session binding in `App.tsx`, and evicting the oldest detached PTY session in `stratad`.
- Reduced `stratad` SideVeil visibility log noise and rebuilt WebUI bundles with `@gojona/ui` tab-swipe and `@gojona/bridge` `WebMessageListener` fixes.

## [0.1.0] - 2026-11-24

### Added

- **Pure-Rust Multi-Container Runtime & Daemon (`crates/`)**:
  - Zero-GKI-modification container runtime targeting stock Android 16+ (`aarch64-linux-android`) with native `/dev/loop-control` ioctls, SELinux xattr labeling, ext4 mounting with `IMAGE_BUSY` duplicate-mount protection, progressive namespace isolation (`NEWNS`, `NEWUTS`, `NEWCGROUP`, `NEWTIME`), minimal `tmpfs /dev`, `pivot_root(".", ".")`, Linux v3 capability bounding, and native `FITRIM` compaction.
  - Zero-polling `pidfd` + Tokio `AsyncFd` container supervision in `stratad`, `inotify` configuration watching, netlink DNS synchronization, dual-signal zero-residual teardown (cgroup v2 + mount namespace inode scan), and UID-0 `SO_PEERCRED`-verified JSON-RPC 2.0 control plane (`/data/adb/strata/run/strata.sock`).
  - Synchronous `strata` CLI supporting multi-container lifecycle, local single-threaded `setns` interactive TTY sessions, headless streaming command/Python execution, device/mount passthrough controls (including opt-in L5 kernel diagnostic mounts), SSH key management, and storage maintenance (`compact`, `fsck`, `backup`).
  - Standalone-first container environment with ColorOS/OPLUS watchdog-compliant key-only Dropbear SSH + SFTP, POSIX `host-exec` (`nsenter -t 1`), and KernelSU module packaging (`build.sh`, `reload.sh`, `customize.sh`, `service.sh`, `post-fs-data.sh`, `uninstall.sh`).
- **Optional SideVeil Plugin Integration & WebUI (`webui/`, `plugins/strata.manager/`)**:
  - Auto-reconnecting SideVeil `hub.sock` plugin backend (`strata.manager`) built on `sideveil-plugin-sdk` with Native HUD scene updates, reliable/ephemeral event tiers, and atomic plugin staging (`scripts/sync-sideveil-plugin.sh`).
  - First-party SideVeil plugin WebUI built with React 19, TypeScript, Tailwind CSS v4, and `@gojona/{ui,bridge,config,sideveil-sdk}` featuring container lifecycle controls, streaming task runner, passthrough/SSH configuration, and a live bridge trace inspector.
- **Contract-First Schema & Codegen (`contracts/`)**:
  - Single-source-of-truth `contracts/strata.contract.json` generating deterministic Rust, TypeScript, and JSON Schema artifacts via `@gojona/contracts-gen` (`pochita-contracts-gen`) alongside 13 golden wire fixtures and cross-stack drift gates.
- **L1–L5 Device Debugging Toolkit & MCP Server (`container/strata-debug/`, `scripts/`)**:
  - In-container `strata-debug` CLI covering WebView CDP WebSocket inspection/audits, incremental logcat cursors, raw screencap visual checks, `/dev/uinput` multitouch gesture replay, UI tree dump/tap, Perfetto ring-buffer capture and protobuf analysis, and LSPosed/KernelSU/SQLite/pstore/bpftrace diagnostics.
  - Multi-device auto-failover SSH MCP server (`scripts/strata-debug-mcp.py`, 27 tools) with per-device ControlMaster isolation and comprehensive host-side unit tests.
- **CI, Release Automation & Licensing**:
  - GitHub Actions CI workflow (`.github/workflows/ci.yml`) covering contract/WebUI, Rust, and Python quality gates, plus automated tagged release builds (`.github/workflows/release.yml`) publishing `strata-<tag>.zip` and `SHA256SUMS.txt`.
  - Licensed under GNU General Public License v3.0 only (`GPL-3.0-only`).

[0.2.0]: https://github.com/5jona/strata/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/5jona/strata/releases/tag/v0.1.0
