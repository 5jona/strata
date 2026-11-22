# Strata — Engineering Standards

See [`docs/architecture-decisions.md`](../docs/architecture-decisions.md) for empirical Android 16+ GKI constraints and container runtime design decisions.

## 1. Standalone Architecture & Boundaries
- **Target Platform**: Android 16+ (API 36+, `aarch64-linux-android`) on stock GKI kernels without kernel patches or `.ko` drivers.
- **Independent Operation**: `stratad`, `strata` CLI, and in-container Dropbear SSH must function without SideVeil installed or running. Any SideVeil integration connects through an optional, auto-reconnecting plugin bridge seam.
- **Pure Logic Separation**: Keep protocol schemas, config validation, capability profile bitmasks, restart backoff state machines, and `/proc`/`/sys` parsers platform-agnostic and unit-tested on the host without root privileges or live sockets.
- **Single Source of Truth**: Declare module IDs, default container names, paths, RPC method/event names, and capability profiles once in `crates/strata-protocol`.

## 2. Modularity, Safety & Runtime Invariants
- **File Size Limits**: Keep every source file at or below **300 lines** (hard limit **400 lines**). Split by cohesion before crossing 400 lines.
- **Rust Workspace Discipline**:
  - Inherit workspace lints via `[lints] workspace = true`.
  - Never create `mod.rs` files; use `foo.rs` with sibling `foo/bar.rs`.
  - Concentrate all `unsafe` blocks in `crates/strata-sys` (or narrowly scoped `strata-runtime` exec calls) with explicit `// SAFETY:` comments.
  - Never call `.unwrap()` or `.expect()` in runtime code paths (tests excluded).
  - Persist all configuration and container runtime files atomically (`temp -> write -> sync_all -> chmod -> rename`).
- **Control Plane Security**:
  - Bind `/data/adb/strata/run/strata.sock` inside a `0700 root:root` directory and verify `uid == 0` via kernel `SO_PEERCRED` on every accepted connection.
  - Pass sensitive launch specs (`Stage2Spec` / `EnterSpec`) over inherited pipes rather than command-line arguments.
- **Single-Threaded Isolation Execution**:
  - Never run complex `unshare`, `mount`, `pivot_root`, or `setns` sequences inside a `fork()` pre-exec hook of a multi-threaded Tokio process. Re-exec a dedicated single-threaded helper mode (`stratad __init` / `stratad __exec`) or execute directly in single-threaded `strata` CLI.
- **Zero-Residual Teardown**:
  - Reject concurrent mounts of the same `(st_dev, st_ino)` backing image on another mountpoint with `IMAGE_BUSY`.
  - Drain container processes via dual signals (container cgroup `cgroup.procs` + `/proc/[0-9]*/ns/mnt` matching verified non-PID-1 mount namespace inode), send `SIGTERM`, wait up to 3s, then `cgroup.kill` + `SIGKILL`, remove container cgroup, recursively unmount deepest-first, and detach all matching loop devices until `mounts == 0 && loops == 0`.
- **OEM & KernelSU Safety**:
  - Never install or run OpenSSH `sshd` inside containers on ColorOS/OPLUS kernels (`oplus_secure_guard_new.ko`); use key-only `dropbear -F -E -s -g -m` + `openssh-sftp-server` and `APT::Sandbox::User "root";`.
  - Never execute Android Bionic binaries directly inside glibc containers; keep in-container bridge helpers (`host-exec`, `strata-debug`) as POSIX sh / Python 3 scripts invoking `nsenter -t 1 -m -u -C -T -r -w`.
  - Unset `ASH_STANDALONE` in KernelSU shell scripts and invoke Android binaries via explicit `/system/bin/*` paths.

## 3. Language, Privacy, Commits & Verification
- **Language & Privacy**: Write all code, identifiers, comments, logs, commit messages, and documentation in objective English. Write user-facing UI copy in Simplified Chinese (`zh-CN`). Never commit device serials, device-specific hardware specs, personal paths, or private network IPs.
- **Commit Messages**: Use Conventional Commits (`<type>(<scope>): <imperative summary>`, <= 72 chars); include 2–5 concise bullet points for multi-file or architectural commits.
- **Verification**: Every change must pass `mise run check` with zero warnings or errors, and `bash build.sh` when packaging changes.
