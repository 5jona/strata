# Strata Architecture Map

This document is the primary architectural navigation map for contributors and AI agents working in `strata`.
For empirical Android 16+ GKI rationale and historical decision records, see `docs/architecture-decisions.md`.
For coding standards and workspace rules, see `AGENTS.md` and `crates/AGENTS.md`.

## 1. System Role & Standalone-First Invariant

Strata is a pure-Rust, multi-container Linux runtime and KernelSU module targeting stock Android 16+ (`aarch64-linux-android`) GKI kernels.
It replaces shell-based `ruri` wrappers with direct Linux syscall orchestration:

- Native `/dev/loop-control` ioctls, SELinux xattr labeling (`u:object_r:vold_data_file:s0`), and ext4 mounting with `(st_dev, st_ino)` duplicate-mount protection (`IMAGE_BUSY`).
- Progressive namespace isolation (`NEWNS`, `NEWUTS`, `NEWCGROUP`, `NEWTIME`), minimal `tmpfs /dev`, `pivot_root(".", ".")`, and Linux v3 capability bounding.
- Zero-polling `pidfd` + Tokio `AsyncFd` supervision in `stratad`, paired with a synchronous CLI (`strata`) for native interactive TTY sessions.

**Standalone-First Invariant**: `stratad`, `strata`, and in-container Dropbear SSH/SFTP operate 100% independently of SideVeil.
SideVeil integration (`strata.manager`) is an optional, auto-reconnecting plugin seam over `/data/system/sideveil/run/hub.sock`.
When SideVeil is absent, stopped, crashed, or disabled in `/data/adb/strata/config.json`, container boot, `strata.sock` JSON-RPC, interactive CLI sessions, and SSH/SFTP access continue without degradation.

```text
[PC AI Agent / Dev Host]
   |-- SSH ControlMaster ---> [Dropbear in Container] ---> host-exec (nsenter -t 1)
   |-- strata-debug-mcp.py -> [strata-debug CLI]      ---> /data/adb/ksu/bin/strata
                                                                 |
                                                    /data/adb/strata/run/strata.sock
                                                                 v
[SideVeil hub.sock (optional)] <--- sideveil_seam ---> [stratad (UID 0 Tokio)]
                                                           |             |
                                             stratad __init|             |stratad __exec
                                                           v             v
                                                  [Container Init]  [Headless Task]
```

## 2. Subsystem & File Ownership Map

### 2.1 Rust Workspace (`crates/`)

Dependency direction is strictly acyclic:
`strata-protocol` (leaf) <- `strata-sys` <- `strata-runtime` <- `stratad` & `strata-ctl`.
Every crate inherits `[lints] workspace = true`, forbids `mod.rs` (uses `foo.rs` + sibling `foo/bar.rs`), and keeps source files `<= 300` lines (hard limit `400`).

#### `crates/strata-protocol/src/` — Platform-Agnostic Schema & Wire Types (Zero Syscalls / Zero Tokio)
- `constants.rs`: Single source of truth for module ID (`strata`), default container (`main`), host/container paths, SELinux label, cgroup v2 root, DNS fallbacks, and opt-in L5 diagnostic mount paths.
- `config.rs`, `config/validate.rs`: Multi-container `StrataConfig` schema, per-container `ContainerConfig`, SideVeil bridge settings, and pure host-tested validation rules.
- `rpc.rs`, `rpc/payloads.rs`, `rpc/tasks.rs`: JSON-RPC 2.0 envelopes (`RpcRequest`, `RpcResponse`, `EventEnvelope`), method/event/error-code constants, and typed request/response payloads.
- `state.rs`: `ContainerPhase` state machine, status/disk/daemon snapshots, and deterministic `RestartBackoff`.
- `capabilities.rs`: Linux capability profile bitmasks (`Standard`, `Privileged`, etc.).

#### `crates/strata-sys/src/` — Audited Linux/Android Syscall Layer
- `loopdev.rs`, `mount.rs`, `devfs.rs`, `selinux.rs`: Loop device attach/detach ioctls, ext4 mount/unmount/`FITRIM`, minimal `tmpfs /dev` and read-only `/sys` fallback, SELinux xattr labeling.
- `namespace.rs`, `caps.rs`, `cgroup.rs`, `pidfd.rs`, `inotify.rs`: `unshare`/`setns`, `capset`, cgroup v2 migration and `cgroup.kill`, `pidfd_open`/`pidfd_send_signal`, and config file watching.
- `net.rs`, `net/netlink.rs`, `procfs.rs`, `error.rs`: Interface IPv4 lookup, netlink address/route events, `/proc` mountinfo/namespace parsers, and typed OS error mapping.

#### `crates/strata-runtime/src/` — Single-Threaded Isolation & Storage Engine
- `stage2.rs`: Single-threaded `stratad __init` bootstrap (cgroup join, `unshare`, `/dev`, `pivot_root`, capability drop, `execve`).
- `enter.rs`: Single-threaded `setns` enter path shared by `stratad __exec` and local `strata` CLI sessions.
- `spec_pipe.rs`: Inherited read-only spec pipe (`Stage2Spec`/`EnterSpec`) and `O_CLOEXEC` pre-exec error pipe.
- `storage.rs`, `storage/maintenance.rs`: Ext4 mount, dual-signal zero-residual teardown, `FITRIM` compaction, `e2fsck`, and gzip image backup.
- `fs_atomic.rs`, `sync.rs`, `sync/debug_tools.rs`: Atomic write-sync-chmod-rename persistence, `/etc/resolv.conf` + `/run/strata/host.env` sync, and `host-exec`/`strata-debug` staging.

#### `crates/stratad/src/` — UID 0 Tokio Supervisor Daemon
- `main.rs`: Pre-Tokio `__init`/`__exec` helper dispatch, daemon startup, signal handling, and clean shutdown.
- `rpc_server.rs`, `dispatcher.rs`: Bounded-frame `strata.sock` server (`SO_PEERCRED` `uid == 0`) and transport-agnostic JSON-RPC router (`RpcCaller::LocalCtl` vs. `RpcCaller::SideVeilBridge`).
- `manager.rs`, `manager/{launch,leases,ops,snapshot,supervisor}.rs`: Multi-container lifecycle serialization, session/temp-mount leases, maintenance operations, status snapshots, and `pidfd` + `AsyncFd` restart supervision.
- `task_runner.rs`, `task_runner/io.rs`: Headless streaming command/Python runner with ordered `task.output` -> `task.exit` delivery and ring-buffer log replay.
- `config_store.rs`, `net_watcher.rs`: Atomic config persistence, shutdown-aware `inotify` reloader, and netlink DNS re-sync trigger.
- `sideveil_seam.rs`, `sideveil_seam/{bridge,session}.rs`, `sideveil_hud.rs`: Dual-channel `EventHub`, `sideveil-plugin-sdk`-backed auto-reconnecting `hub.sock` NDJSON bridge (`PluginBackendRunner` / `PluginBackendHandle`), and Native HUD publisher.

#### `crates/strata-ctl/src/` — Synchronous CLI (`strata`)
- `main.rs`, `client.rs`: Synchronous Unix socket NDJSON client talking to `/data/adb/strata/run/strata.sock` with streaming event output.
- `commands.rs`, `commands/config.rs`, `format.rs`: Subcommands for `start`, `stop`, `restart`, `status`, `list`, `logs`, `dns`, `ssh-key`, `compact`, `fsck`, `backup`, `daemon`, and `config` patching (including `set-l5-diag`).
- `interactive.rs`: Local single-threaded `setns` TTY execution for `strata enter|exec|py|debug` after acquiring a connection-scoped lease via `session.acquire`.
- Installed via KernelSU symlink at `/data/adb/ksu/bin/strata` pointing to `${MODDIR}/bin/strata`.

### 2.2 WebUI, Container Tooling, MCP & KernelSU Scripts

| Subsystem | Paths | Responsibility |
| --- | --- | --- |
| SideVeil Plugin UI | `webui/`, `plugins/strata.manager/` | React 19 + TS + Tailwind v4 frontend (`StrataManagerApp.tsx`, `components/`, `hooks/`, `lib/strataRpc.ts` delegating to `@gojona/sideveil-sdk`'s `createTypedPluginClient`), built into `plugins/strata.manager/web/` with `manifest.json` (`uds-native`, `toast`/`clipboard`/`haptics`/`native-hud`). |
| In-Container Bridge | `container/host-exec`, `container/setup-container.sh` | POSIX `nsenter -t 1 -m -u -C -T -r -w` host bridge and Ubuntu rootfs provisioning helper. |
| Device Debug CLI | `container/strata-debug/` | Python 3 toolkit (`cli.py`, `cli_android.py`, `cdp_*`, `logcat_cursor.py`, `gesture_replay.py`, `visual_check.py`, `observe_step.py`, `app_ops.py`, `ui_tree.py`, `lsposed_ops.py`, `host_inspect.py`, `perfetto_*`, `kernel_probe.py`, `sys_probe.py`, `forward_bridge.py`). |
| PC-Side MCP Server | `scripts/strata-debug-mcp.py`, `scripts/strata_mcp/` | Stdio JSON-RPC 2.0 MCP server exposing 27 tools over SSH ControlMaster (`transport.py`, `transport_routing.py`, `devices.py`, `device_discovery.py`, `tools_core.py`, `tools_android.py`, `tools_devices.py`). |
| Live Verification | `scripts/verify-live.py`, `scripts/verify_live/` | On-device integration checks (`runner.py`, `core_checks.py`, `debug_checks.py`, `sideveil_checks.py`) over `auto\|ssh\|adb\|host-exec\|local`. |
| KernelSU Lifecycle | `post-fs-data.sh`, `service.sh`, `customize.sh`, `uninstall.sh`, `scripts/sync-sideveil-plugin.sh` | Early directory/symlink setup, daemon supervision loop, module install/uninstall hooks, and plugin asset staging. |

## 3. Core Runtime Flows

### 3.1 Boot, Stage-2 Init & `pidfd`/`AsyncFd` Supervision
1. `post-fs-data.sh` creates `/data/adb/strata/run` (`0700 root:root`), default mount dir `/data/local/strata/containers/main/mnt`, and `/data/adb/ksu/bin/strata`.
2. `service.sh` waits for `sys.boot_completed=1` (skipped with `--now`), seeds `/data/adb/strata/config.json`, stages `plugins/strata.manager`, and supervises `stratad`.
3. `stratad` checks `argv[1]` **before** creating any Tokio runtime so `__init` and `__exec` run purely single-threaded (`ADR-002`).
4. On `container.start` (`manager/launch.rs`), `stratad` attaches the ext4 image via `storage.rs` (rejecting duplicate `(st_dev, st_ino)` mounts with `IMAGE_BUSY`), syncs `/etc/resolv.conf`, `/run/strata/host.env`, `host-exec`, and `strata-debug`, then spawns `stratad __init --spec-fd <fd> --err-fd <fd>`.
5. `stage2.rs` joins `/sys/fs/cgroup/strata/<id>`, calls `unshare(CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWCGROUP | CLONE_NEWTIME)`, builds minimal `tmpfs /dev` + `tmpfs /dev/shm`, isolates via `pivot_root(".", ".")` + `umount2(".", MNT_DETACH)`, drops capabilities, and `execve`s container init (default: Dropbear). The `O_CLOEXEC` error pipe reports any pre-exec failure synchronously.
6. `manager/supervisor.rs` wraps `pidfd_open(init_pid)` in Tokio `AsyncFd` for zero-polling exit notification, scheduling exponential `RestartBackoff` restarts while `desired_running == true`.

### 3.2 Interactive `session.acquire` vs. Headless `TaskRunner`
- **Interactive CLI (`strata enter|exec|py|debug`)**: Calls `session.acquire` on `strata.sock` (restricted to `RpcCaller::LocalCtl`; denied over `SideVeilBridge`). `manager/leases.rs` tracks connection-scoped leases and, for `Oneshot`/`Debug` modes on stopped containers, mounts the image temporarily. The single-threaded `strata` binary then executes `strata_runtime::enter` locally for full PTY, `SIGWINCH`, and job-control fidelity, releasing the lease on exit or automatically on socket disconnect.
- **Headless Streaming (`task.run`)**: Accessible over both `strata.sock` and SideVeil `hub.sock`. `TaskRunner` spawns `stratad __exec`, captures stdout/stderr lines into a bounded ring buffer (`task.logs`) and `EventHub` (`task.output`), awaits both stream readers to EOF before emitting `task.exit`, and supports `task.cancel` via `pidfd_send_signal(SIGTERM)`.

### 3.3 Dual-Signal Zero-Residual Teardown
Because stock GKI omits `PID_NS`, terminating Dropbear does not kill detached background processes (`ADR-004`). `storage.rs::teardown_container`:
1. Migrates the teardown thread out of any `/strata/` cgroup.
2. Collects PIDs from `/sys/fs/cgroup/strata/<id>/cgroup.procs` and from `/proc/[0-9]*/ns/mnt` matching the container mount namespace inode (verified distinct from PID 1's mount namespace, excluding PID 1 and `stratad`).
3. Sends `SIGTERM`, waits up to 3s for exit, then escalates to `cgroup.kill` + `SIGKILL`.
4. Removes the container cgroup, unmounts all mountpoints deepest-first, and detaches matching loop devices until `mounts == 0 && loops == 0`.

### 3.4 Container Lifecycle State Machine & On-Device Paths

```text
Stopped / Failed ---> Starting ---> Running ---> Stopping ---> Stopped
                          |             |            |
                          +-------------+------------+-------> Failed
```

| On-Device Path | Owner & Mode | Role |
| --- | --- | --- |
| `/data/adb/strata/config.json` | `0600 root:root` | Authoritative multi-container and SideVeil bridge configuration (watched via `inotify`). |
| `/data/adb/strata/run/strata.sock` | `0700` dir, UID 0 | Standalone NDJSON JSON-RPC 2.0 control plane (`SO_PEERCRED` verified). |
| `/data/adb/strata/run/logs/` | `0700 root:root` | Daemon stdout/stderr (`stratad.log`) and rotated per-container init logs (`<id>-sshd.log`). |
| `/data/local/strata/containers/<id>/` | `0755 root:root` | Container ext4 rootfs image and mountpoint (`mnt/`). |
| `/data/adb/ksu/bin/strata` | symlink | Global PATH entry pointing to `${MODDIR}/bin/strata`. |
| `/sys/fs/cgroup/strata/<id>` | cgroup v2 | Per-container process tracking and `cgroup.kill` teardown handle. |
| `/run/strata/host.env` (in-container) | `0644` | Atomically synced Android PID 1 environment snapshot for `host-exec`. |

## 4. Protocol Boundaries

### 4.1 `strata.sock` JSON-RPC 2.0 Control Plane
- **Endpoint**: `/data/adb/strata/run/strata.sock` inside `0700 root:root`; every connection is authenticated via `SO_PEERCRED` (`uid == 0`).
- **Framing**: NDJSON requests/responses and server-pushed `EventEnvelope` frames, bounded to `MAX_FRAME_BYTES` before heap growth.
- **Method Families**:
  - `daemon.*`: `ping`, `status`, `health`, `reloadConfig`
  - `container.*`: `list`, `status`, `start`, `stop`, `restart`, `logs`, `syncDns`, `compact`, `fsck`, `backup`, `backupStart`, `configGet`, `configPatch`, `sshKeyList`, `sshKeyAdd`
  - `session.*` (`LocalCtl` only): `acquire`, `release`
  - `task.*`: `run`, `cancel`, `list`, `logs`
  - `hud.*` & events: `hud.state`, `hud.set`, `events.subscribe`
- **Domain Error Codes** (`crates/strata-protocol/src/rpc.rs`):
  - Standard JSON-RPC: `-32700` (`PARSE_ERROR`), `-32600` (`INVALID_REQUEST`), `-32601` (`METHOD_NOT_FOUND`), `-32602` (`INVALID_PARAMS`).
  - Strata runtime: `-32000` (`INTERNAL_ERROR`), `-32003` (`PERMISSION_DENIED`), `-32020` (`IMAGE_BUSY`), `-32021` (`CONTAINER_NOT_FOUND`), `-32022` (`CONTAINER_ALREADY_RUNNING`), `-32023` (`CONTAINER_NOT_RUNNING`), `-32024` (`INVALID_STATE`), `-32025` (`TASK_NOT_FOUND`).

### 4.2 SideVeil `hub.sock` & Native HUD
- **Endpoint**: `/data/system/sideveil/run/hub.sock` via `sideveil_seam/{bridge,session}.rs`, backed by `sideveil-plugin-sdk` (`PluginBackendRunner`, `PluginBackendHandle`, and bounded `LinesCodec` NDJSON framing).
- **Protocol**: NDJSON `pluginBackend` handshake, multiplexed JSON-RPC 2.0 (routed into `Dispatcher` as `RpcCaller::SideVeilBridge`, with `session.*` rejected), Hub system notifications (`appearance.changed`, `panel.visibilityChanged`, `network.contextChanged`), `plugin.event`, and Native HUD `hud.updateScene` / `hud.dismiss` (`sideveil_hud.rs`).
- **Backpressure Isolation**: Dual-channel `EventHub` and `sideveil-plugin-sdk` outbound queues separate reliable lifecycle events (`container.stateChanged`, `container.maintenanceDone`, `task.exit`) from ephemeral high-volume `task.output` and periodic HUD refreshes.
- **Session Lifecycle**: `PluginBackendRunner` backs off automatically when SideVeil is absent or restarting, `bridge.rs` wakes immediately on config reload (`notify_config_changed`), and shutdown/disable emits `hud.dismiss` before cancelling the session token.

### 4.3 `strata-debug` <-> 27-Tool MCP Server
- **Transport**: PC stdio JSON-RPC 2.0 (`scripts/strata-debug-mcp.py`) <-> SSH ControlMaster (`strata_mcp/transport.py`) with multi-device auto-failover (`devices.py`, `transport_routing.py`) <-> in-container `strata-debug` (`container/strata-debug/cli.py`).
- **Tool Coverage (27 tools)**:
  - Multi-device & core (`tools_devices.py`, `tools_core.py`): device discovery/select, health snapshot, hot-reload/verify, L1 CDP audit/eval/console/targets/forward, L2a incremental logcat cursor, L2b `/dev/uinput` gesture replay, L3 visual screencap checks, L4 atomic `observe_step`, Perfetto ring-buffer/analyze, L5 pstore/kernel diagnostics/probes.
  - Android host & container (`tools_android.py`): app control/inspect, APK install, UI tree dump/tap, SQLite read-only query, LSPosed inspect, KernelSU inspect, file transfer, and arbitrary host/container exec.

## 5. Android 16+ GKI & ColorOS Watchdog Invariants

- **Zero GKI Kernel Dependencies** (`ADR-003`): Never require `PID_NS`, `IPC_NS`, `SYSVIPC`, `USER_NS`, `DEVTMPFS`, or out-of-tree `.ko` drivers. Default containers mount minimal `tmpfs /dev` (hiding `/dev/block/*`) and remount fallback host `/sys` bind-mounts read-only (`MS_REMOUNT | MS_BIND | MS_RDONLY`).
- **ColorOS Security Watchdog Compliance** (`ADR-005`): Never install or run OpenSSH `sshd` privsep inside containers on ColorOS/OPLUS kernels (`oplus_secure_guard_new.ko`). Always use key-only `dropbear -F -E -s -g -m` + `openssh-sftp-server` and `APT::Sandbox::User "root";`.
- **Cross-Boundary Execution Hygiene**: Never invoke Android Bionic binaries directly from glibc containers; route host commands through POSIX `host-exec` (`nsenter -t 1`). In KernelSU shell scripts, `unset ASH_STANDALONE` and use explicit `/system/bin/*` absolute paths.

## 6. Concurrency, Lock Hierarchy & Safety Rules

- **Strict Lock Hierarchy**: Always acquire locks in the order `slot.lifecycle_lock` -> `slot.state` -> `manager.leases`. Never hold the `manager.slots` map lock across lifecycle `.await` points.
- **Continuous Maintenance Serialization**: Hold `slot.lifecycle_lock` continuously across stop + maintenance (`stop_locked` + `e2fsck` / `backup_image_gzip`) so concurrent `container.start` cannot mount the ext4 image mid-operation.
- **Lease TOCTOU & Active-Lease Guards**: Inside `cleanup_temp_lease_if_idle`, re-check `has_temp_mount_lease()` *after* acquiring `slot.lifecycle_lock`. Reject `start_container` and `compact_container` while temporary `Oneshot`/`Debug` mount leases are active.
- **`spawn_blocking` Offloading**: Execute blocking syscalls (loop ioctls, ext4 mount/teardown, `e2fsck`, `FITRIM`, gzip backup, `inotify` reads, exec-error pipe reads) on `tokio::task::spawn_blocking`.
- **Cancellation Safety**: Async NDJSON framing over SideVeil `hub.sock` uses `sideveil-plugin-sdk`'s cancellation-safe `Framed<UnixStream, LinesCodec>`, and `strata.sock` readers polled inside `tokio::select!` retain caller-owned line buffers outside `select!` so winning arms never drop partial frames.
- **Zero-Panic, Atomic I/O & Cleanup**: Zero `.unwrap()`/`.expect()`/`panic!` outside `#[cfg(test)]`. Persist config/runtime files atomically (`fs_atomic.rs`), filter self-written `.tmp` inotify events, wire shutdown signals into background watchers, and kill + reap spawned helper `Child` handles on early error paths.

## 7. Quality Gates & Deploy Workflows

| Workflow | Command | Scope |
| --- | --- | --- |
| Full Quality Gate | `mise run check` | `rust:check` (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`), `webui:check` (`pnpm check`), `python:check` (`py_compile` + `unittest discover -s tests -v`). |
| Android Cross-Build | `mise run build` | Release `aarch64-linux-android` binaries (`stratad`, `strata`) via `scripts/aarch64-android-linker.sh`. |
| Module Archive | `bash build.sh` | Runs all quality gates, cross-compiles binaries, stages `bin/`, and packages `out/strata.zip` with exact POSIX modes. |
| Hot-Reload Deploy | `bash reload.sh [--ssh <target> \| --adb <serial> \| --local]` | Builds (unless `--skip-build`), atomically stages module files and plugin assets, and restarts `stratad` without rebooting. |
| Live Verification | `python3 scripts/verify-live.py [--mode ... \| --ui-only]` | Verifies container runtime, SideVeil backend bridge, and live WebView rendering on-device. |

### Quick Change-Routing Index
- **Adding or changing an RPC method/event**: Update `crates/strata-protocol/src/rpc*.rs` -> `crates/stratad/src/dispatcher.rs` -> `crates/strata-ctl/src/commands*.rs` and `webui/src/types.ts` + `webui/src/hooks/`.
- **Changing container isolation, mounts, or `/dev`**: Update `crates/strata-sys/src/{devfs,mount,namespace,caps}.rs` -> `crates/strata-runtime/src/{stage2,enter,storage}.rs` -> verify host tests and GKI invariants (`ADR-003`).
- **Changing config schema or defaults**: Update `crates/strata-protocol/src/config*.rs` -> `config.json` -> `crates/stratad/src/config_store.rs` -> `crates/strata-ctl/src/commands/config.rs` -> `webui/src/components/ConfigTab.tsx`.
- **Adding a device debugging tool**: Implement subcommand in `container/strata-debug/` -> wire CLI parser in `cli.py` / `cli_android.py` -> expose MCP tool in `scripts/strata_mcp/tools_*.py` -> add host unit tests in `tests/`.
- **Updating SideVeil WebUI assets**: Edit `webui/src/`, run `mise run webui:check` (rebuilds `plugins/strata.manager/web/`), and verify `scripts/sync-sideveil-plugin.sh` staging.
