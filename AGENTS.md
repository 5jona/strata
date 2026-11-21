# Strata — Agent Guidelines

Read `.agent/rules.md` for complete engineering standards and `docs/architecture-decisions.md` for Android 16+ kernel constraints and container runtime invariants.

## Architecture & Crate Boundaries
- **`crates/strata-protocol`**: Platform-agnostic multi-container config schema, JSON-RPC 2.0 wire types, lifecycle state machine, capability bitmask profiles, and shared constants. Zero Linux syscalls or Tokio dependencies.
- **`crates/strata-sys`**: Minimal audited surface for Linux/Android syscalls (`loop-control`, `mount`, `pivot_root`, `FITRIM`, `unshare`, `setns`, `capset`, `pidfd`, `inotify`, `SO_PEERCRED`, `getifaddrs`) and `/proc`/`/sys` parsers.
- **`crates/strata-runtime`**: Single-threaded stage-2 container initializer, single-threaded `setns` enter path, storage mount/zero-residual teardown/maintenance engine, and atomic file sync.
- **`crates/stratad`**: UID 0 Tokio daemon owning multi-container supervision (`pidfd` + `AsyncFd`, zero polling), `inotify` config watching, `/data/adb/strata/run/strata.sock` control plane, headless streaming task runner, and optional SideVeil plugin bridge seam.
- **`crates/strata-ctl`**: Synchronous CLI binary (`strata`, symlinked to `/data/adb/ksu/bin/strata`) supporting multi-container management and local `setns` interactive TTY sessions.

## Non-Negotiable Invariants
- **Standalone Availability**: `stratad`, `strata`, and Dropbear SSH must run 100% independently when SideVeil is absent, stopped, or crashed.
- **Zero GKI / Kernel-Module Dependency**: Never require `PID_NS`, `IPC_NS`, `SYSVIPC`, `USER_NS`, `DEVTMPFS`, or out-of-tree `.ko` modules.
- **ColorOS Security Watchdog Compliance**: Never run OpenSSH `sshd` privsep inside containers; use key-only `dropbear -F -E -s -g -m` + `openssh-sftp-server` and `APT::Sandbox::User "root";`.
- **File Size & Rust Discipline**: Keep every source file <= 300 lines (hard limit 400 lines). Never use `mod.rs`. Document every `unsafe` block with `// SAFETY:`. Never use `.unwrap()` or `.expect()` in runtime paths. Persist files atomically (`temp -> fsync -> rename`).
- **KernelSU Script Hygiene**: Thin `service.sh` scripts must `unset ASH_STANDALONE` and use explicit `/system/bin/*` absolute paths.
- **Language & Privacy**: Objective English for code, comments, logs, commit messages, and docs; Simplified Chinese (`zh-CN`) for user-facing UI copy. Never commit device serials, hardware specs, personal paths, or private IPs.

## Build & Verification Commands
- Quality gate: `mise run check` (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`).
- Android cross-build: `mise run build` (`aarch64-linux-android`).
- KernelSU module archive: `bash build.sh` (outputs `out/strata.zip`).
