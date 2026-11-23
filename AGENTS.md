# Strata — Agent Guidelines

For empirical Android 16+ (API 36+, `aarch64-linux-android`) stock GKI constraints and container runtime design decisions, see `docs/architecture-decisions.md`. Rust crate-level rules live in `crates/AGENTS.md`.

## Architecture & Crate Boundaries
- **`crates/strata-protocol`**: Platform-agnostic multi-container config schema, JSON-RPC 2.0 wire types, lifecycle state machine, capability bitmask profiles, and single-source-of-truth constants (module IDs, default container names, paths, RPC methods/events). Zero Linux syscalls or Tokio dependencies.
- **`crates/strata-sys`**: Minimal audited surface for Linux/Android syscalls (`loop-control`, `mount`, `pivot_root`, `FITRIM`, `unshare`, `setns`, `capset`, `pidfd`, `inotify`, `SO_PEERCRED`, `getifaddrs`) and `/proc`/`/sys` parsers.
- **`crates/strata-runtime`**: Single-threaded stage-2 container initializer, single-threaded `setns` enter path, storage mount/zero-residual teardown/maintenance engine, and atomic file sync.
- **`crates/stratad`**: UID 0 Tokio daemon owning multi-container supervision (`pidfd` + `AsyncFd`, zero polling), `inotify` config watching, `/data/adb/strata/run/strata.sock` control plane, headless streaming task runner, and optional auto-reconnecting SideVeil plugin bridge seam.
- **`crates/strata-ctl`**: Synchronous CLI binary (`strata`, symlinked to `/data/adb/ksu/bin/strata`) supporting multi-container management and local `setns` interactive TTY sessions.

## Non-Negotiable Invariants
- **Standalone Availability**: `stratad`, `strata` CLI, and in-container Dropbear SSH must run 100% independently when SideVeil is absent, stopped, or crashed.
- **Zero GKI / Kernel-Module Dependency**: Target stock Android 16+ GKI kernels without kernel patches or `.ko` drivers. Never require `PID_NS`, `IPC_NS`, `SYSVIPC`, `USER_NS`, `DEVTMPFS`, or out-of-tree modules.
- **ColorOS Security Watchdog Compliance**: Never install or run OpenSSH `sshd` privsep inside containers on ColorOS/OPLUS kernels (`oplus_secure_guard_new.ko`); use key-only `dropbear -F -E -s -g -m` + `openssh-sftp-server` and `APT::Sandbox::User "root";`.
- **Zero-Residual Teardown**: Reject concurrent mounts of the same `(st_dev, st_ino)` backing image on another mountpoint with `IMAGE_BUSY`. Drain container processes via dual signals (container cgroup `cgroup.procs` + verified non-PID-1 `/proc/[0-9]*/ns/mnt` inode), send `SIGTERM`, wait up to 3s, then `cgroup.kill` + `SIGKILL`, remove container cgroup, recursively unmount deepest-first, and detach matching loop devices until `mounts == 0 && loops == 0`.
- **Container & KernelSU Script Hygiene**: Never execute Android Bionic binaries directly inside glibc containers; keep in-container bridge helpers (`host-exec`, `strata-debug`) as POSIX sh / Python 3 scripts invoking `nsenter -t 1 -m -u -C -T -r -w`. In KernelSU shell scripts (`service.sh`, etc.), `unset ASH_STANDALONE` and use explicit `/system/bin/*` absolute paths.
- **File Size, Language & Privacy**: Keep every source file `<= 300` lines (hard limit `400`). Objective English for code, comments, logs, commit messages, and docs; Simplified Chinese (`zh-CN`) for user-facing UI copy. Never commit device serials, hardware-specific specs, personal paths, or private IPs.
- **Commit Messages**: Conventional Commits (`<type>(<scope>): <imperative summary>`, `<= 72` chars); add 2–5 concise bullets for multi-file or architectural commits.

## Build & Verification Commands
- Full quality gate: `mise run check` (`rust:check`, `webui:check`, `python:check`) with zero warnings or errors.
- Android cross-build: `mise run build` (`aarch64-linux-android`).
- KernelSU module archive: `bash build.sh` (outputs `out/strata.zip`).
