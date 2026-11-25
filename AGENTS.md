# Strata — Agent Guidelines

For the codebase navigation map, runtime flows, lock hierarchy, and protocol boundaries, see `docs/ARCHITECTURE.md`. For empirical Android 16+ (API 36+, `aarch64-linux-android`) stock GKI constraints and container runtime design decisions, see `docs/architecture-decisions.md`. Subsystem rules live in `daemon/crates/AGENTS.md` and `desktop/AGENTS.md`.

## Architecture & Crate Boundaries
- **`daemon/crates/strata-protocol`**: Platform-agnostic multi-container config schema, JSON-RPC 2.0 wire types, lifecycle state machine, capability bitmask profiles, and single-source-of-truth constants (module IDs, default container names, paths, RPC methods/events). Zero Linux syscalls or Tokio dependencies.
- **`daemon/crates/strata-sys`**: Minimal audited surface for Linux/Android syscalls (`loop-control`, `mount`, `pivot_root`, `FITRIM`, `unshare`, `setns`, `capset`, `pidfd`, `inotify`, `SO_PEERCRED`, `openat2`, `pty`, `getifaddrs`) and `/proc`/`/sys` parsers.
- **`daemon/crates/strata-runtime`**: Single-threaded stage-2 container initializer, single-threaded `setns` enter path, storage mount/zero-residual teardown/maintenance/provisioning engine, canonical layout sync, and atomic file sync.
- **`daemon/crates/stratad`**: UID 0 Tokio daemon owning multi-container supervision (`pidfd` + `AsyncFd`, zero polling), managed services, `inotify` config watching, `/data/adb/strata/run/strata.sock`, Bearer-authenticated HTTP/WebSocket control plane (`web/`), headless streaming task runner, container tool management (`tools/`), and optional SideVeil plugin bridge seam.
- **`daemon/crates/strata-ctl`**: Synchronous CLI binary (`strata`, symlinked to `/data/adb/ksu/bin/strata`) supporting multi-container management and local `setns` interactive TTY sessions.
- **`tools/strata-debug-agent` & `tools/strata-mcp`**: Standalone pure-Rust static musl on-device debugging agent (`strata-debug`, compiled for `aarch64-unknown-linux-musl` with zero `PT_INTERP`) and PC-side async Rust MCP server (`strata-mcp`).
- **`module/` & `webui/` & `desktop/`**: KernelSU module static sources (`module/`), React 19 + TypeScript + `@gojona/*` UIs built into `out/stage/plugins/strata.manager/web/` and `out/stage/webroot/`, and cross-platform Tauri v2 PC companion (`desktop/` + `desktop/crates/strata-desktop-core`).

## Non-Negotiable Invariants
- **Standalone Availability**: `stratad`, `strata` CLI, and in-container Dropbear SSH must run 100% independently when SideVeil is absent, stopped, or crashed.
- **Zero GKI / Kernel-Module Dependency**: Target stock Android 16+ GKI kernels without kernel patches or `.ko` drivers. Never require `PID_NS`, `IPC_NS`, `SYSVIPC`, `USER_NS`, `DEVTMPFS`, or out-of-tree modules.
- **ColorOS Security Watchdog Compliance**: Never install or run OpenSSH `sshd` privsep inside containers on ColorOS/OPLUS kernels (`oplus_secure_guard_new.ko`); use key-only `dropbear -F -E -s -g -m` + `openssh-sftp-server` and `APT::Sandbox::User "root";`.
- **Zero-Residual Teardown**: Reject concurrent mounts of the same `(st_dev, st_ino)` backing image on another mountpoint with `IMAGE_BUSY`. Drain container processes via dual signals (container cgroup `cgroup.procs` + verified non-PID-1 `/proc/[0-9]*/ns/mnt` inode), send `SIGTERM`, wait up to 3s, then `cgroup.kill` + `SIGKILL`, remove container cgroup, recursively unmount deepest-first, and detach matching loop devices until `mounts == 0 && loops == 0`.
- **Container & KernelSU Script Hygiene**: Never execute Android Bionic binaries directly inside glibc containers; keep `module/container/host-exec` as POSIX sh invoking `nsenter -t 1 -m -u -C -T -r -w`, and keep `strata-debug` as a fully static musl ELF binary (zero `PT_INTERP`). In KernelSU shell scripts (`module/service.sh`, etc.), `unset ASH_STANDALONE` and use explicit `/system/bin/*` absolute paths.
- **Version & Deploy Parity**: Keep `module/module.prop` (`versionCode = MAJOR*10000 + MINOR*100 + PATCH`), `daemon/Cargo.toml`, `tools/strata-debug-agent/Cargo.toml`, `tools/strata-mcp/Cargo.toml`, `webui/package.json`, `module/plugins/strata.manager/manifest.json`, `webui/src/manifest.ts`, `desktop/`, and `docs/CHANGELOG.md` aligned (managed by `scripts/bump-version.mjs` and enforced by `webui/test/contracts.test.mjs`). Keep `scripts/reload.sh` sync lists aligned with `scripts/build.sh`.
- **File Size, Language & Privacy**: Keep every source file `<= 300` lines (hard limit `400`). Objective English for code, comments, logs, commit messages, and docs; Simplified Chinese (`zh-CN`) for user-facing UI copy. Never commit device serials, hardware-specific specs, personal paths, or private IPs.
- **Commit Messages**: Conventional Commits (`<type>(<scope>): <imperative summary>`, `<= 72` chars); add 2–5 concise bullets for multi-file or architectural commits.

## Build & Verification Commands
- Full quality gate: `mise run check` (`version:check`, `contracts:check`, `rust:check`, `webui:check`, `desktop:check`) with zero warnings or errors.
- Individual cross-builds: `mise run build:daemon`, `mise run build:debug-agent`, `mise run build:mcp`.
- KernelSU module archive: `mise run build` or `bash scripts/build.sh` (outputs `out/strata.zip`).
- Device hot-reload: `mise run reload` or `bash scripts/reload.sh`.
