# Strata

Strata is a standalone, zero-GKI-modification, pure-Rust multi-container Linux runtime and KernelSU module for Android 16+ (`arm64`). It replaces shell-based `ruri` wrappers with native Linux syscall orchestration (`stratad` + `strata` CLI) and aligns its JSON-RPC 2.0 control plane for seamless SideVeil plugin integration (`strata.manager`).

## Key Capabilities

- **Pure-Rust Container Runtime**: Direct `/dev/loop-control` ioctls, SELinux xattr labeling, `ext4` mounting with concurrent-mount (`IMAGE_BUSY`) protection, progressive namespace isolation (`NEWNS`, `NEWUTS`, `NEWCGROUP`, `NEWTIME`), minimal `tmpfs /dev` (hiding `/dev/block/*` by default), `pivot_root(".", ".")`, Linux v3 capability bounding, and native `FITRIM` compaction.
- **Zero-Polling Supervision & Clean Teardown**: `pidfd_open` + Tokio `AsyncFd` monitors container init processes without polling; dual-signal teardown (cgroup v2 + mount namespace inode scan) guarantees `mounts == 0 && loops == 0` without requiring `PID_NS`.
- **Multi-Container Control Plane**: `/data/adb/strata/run/strata.sock` (`0700 root:root`, `SO_PEERCRED` `uid == 0`) serves JSON-RPC 2.0 / NDJSON for CLI management, headless streaming command/Python execution, and future SideVeil plugin integration.
- **Native Interactive CLI**: `strata` (symlinked at `/data/adb/ksu/bin/strata`) supports `start`, `stop`, `restart`, `status`, `list`, `enter`, `exec`, `py`, `debug`, `dns`, `logs`, `ssh-key`, `compact`, `fsck`, `backup`, and `config`, using local single-threaded `setns` for zero-overhead interactive TTY sessions.
- **Android Host Bridge**: Syncs `/etc/resolv.conf`, `/run/strata/host.env`, POSIX `host-exec`, and the Python `strata-debug` toolkit without invoking Bionic binaries inside glibc containers.

## Workspace Layout

- `crates/strata-protocol`: Config schema, JSON-RPC wire types, state machine, capability profiles, and constants.
- `crates/strata-sys`: Audited Linux/Android syscall wrappers and `/proc`/`/sys` parsers.
- `crates/strata-runtime`: Single-threaded stage-2 init, `setns` enter, storage engine, and atomic sync.
- `crates/stratad`: UID 0 Tokio daemon, `strata.sock` control plane, and optional auto-reconnecting SideVeil `hub.sock` plugin backend (`strata.manager`) with Native HUD support.
- `crates/strata-ctl`: Multi-container CLI binary (`strata`).
- `container/`: In-container POSIX/Python bridge scripts and Ubuntu provisioning helper.
- `webui/` & `plugins/strata.manager/`: First-party SideVeil plugin frontend (React 19 + TypeScript + Tailwind v4 + `@pochita/ui` + `@pochita/bridge` + `@pochita/sideveil-sdk`).
- `docs/upstream-backlog.md`: Generic upstream improvements identified for `sideveil` and `pochita-webui-kit`.

## Build & Verification

```sh
mise run check   # Rust fmt/clippy/tests + WebUI lint/typecheck/build/standards tests
mise run build   # cross-compile aarch64-linux-android release binaries
bash build.sh    # run full quality gates and package out/strata.zip
```
