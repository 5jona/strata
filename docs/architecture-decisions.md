# Strata — Architecture Decisions

This document captures empirical Android 16+ GKI constraints and the core architectural decisions behind Strata's pure-Rust container runtime.

## ADR-001: Pure-Rust Syscall Runtime Replacing Third-Party `ruri`
- **Context**: Shell-orchestrated wrappers around third-party `ruri` rely on immutable `.rurienv` files inside the ext4 rootfs, external BusyBox/toybox flag conventions, and wrapper processes that complicate lifecycle supervision and multi-container orchestration.
- **Decision**: Implement container storage (`loop-control` ioctls, `mount`, `umount2`, `FITRIM`), namespace isolation (`unshare`, `pivot_root`, `setns`), minimal `/dev` creation, cgroup management, capability bounding, and `pidfd` supervision natively in Rust (`strata-sys` + `strata-runtime` + `stratad`).
- **Consequences**: Zero `.rurienv` artifacts, instant `O_CLOEXEC` exec-sync readiness reporting, zero-polling `pidfd` + `AsyncFd` supervision, and native concurrent-mount protection (`IMAGE_BUSY`).

## ADR-002: Single-Threaded Re-Exec Helper for `unshare` / `pivot_root` / `setns`
- **Context**: Multi-threaded Tokio daemons cannot safely execute complex Rust allocations, path resolution, or `setns(CLONE_NEWNS)` after `fork()` (both because `setns(CLONE_NEWNS)` rejects threads sharing `fs_struct` and because post-fork code is constrained to async-signal-safe syscalls).
- **Decision**: `stratad` spawns itself in single-threaded helper modes (`stratad __init` and `stratad __exec`) before constructing any Tokio runtime. Launch specifications (`Stage2Spec` / `EnterSpec`) are passed over an inherited read-only pipe, while an `O_CLOEXEC` error pipe reports pre-`execve` failures immediately without polling. Interactive CLI sessions (`strata enter|exec|py`) run in the single-threaded `strata` client directly via `session.acquire` + local `setns`.
- **Consequences**: Zero post-fork deadlock risk, native TTY/`SIGWINCH`/job-control fidelity in interactive CLI sessions, and unified streaming task execution for daemon RPC and SideVeil WebUI clients.

## ADR-003: Stock GKI Progressive Isolation Without `PID_NS`, `IPC_NS`, `USER_NS`, or `DEVTMPFS`
- **Context**: Stock Android 16 GKI kernels omit `CONFIG_PID_NS`, `CONFIG_IPC_NS`, `CONFIG_SYSVIPC`, `CONFIG_USER_NS`, and `DEVTMPFS` inside mount namespaces, while supporting `CLONE_NEWNS`, `CLONE_NEWUTS`, `CLONE_NEWCGROUP`, and `CLONE_NEWTIME`.
- **Decision**: Move the container process into `/sys/fs/cgroup/strata/<id>` prior to `unshare(CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWCGROUP | CLONE_NEWTIME)` (gracefully degrading if `CLONE_NEWTIME` is unavailable), mount `tmpfs` at `/dev` with a minimal safe character-device set (hiding `/dev/block/*` by default), mount `mode=1777` `tmpfs /dev/shm`, bind `/dev/pts` and `/data/local/tmp`, and isolate via `pivot_root(".", ".")` + `umount2(".", MNT_DETACH)`.
- **Consequences**: Zero GKI kernel patches or `.ko` modules required. Full host `/dev`, `/sys`, and `/proc` exposure remains opt-in via explicit `debug` or `hostRuntime` mode.

## ADR-004: Dual-Signal Zero-Residual Process Draining Without `PID_NS`
- **Context**: Without a dedicated PID namespace, terminating the container's main process (Dropbear) does not automatically terminate detached background jobs or interactive shells.
- **Decision**: During teardown, first move the teardown thread out of any `/strata/` cgroup, then gather container PIDs from both `/sys/fs/cgroup/strata/<id>/cgroup.procs` and a scan of `/proc/[0-9]*/ns/mnt` matching the container's mount namespace inode (verified distinct from PID 1's mount namespace, excluding PID 1 and `stratad`). Send `SIGTERM`, wait up to 3 seconds for exit, then trigger `cgroup.kill` and `SIGKILL` on any survivors before removing the cgroup, unmounting deepest-first, and detaching all matching loop devices.
- **Consequences**: Guarantees deterministic `mounts == 0 && loops == 0` teardown even under active background workloads.

## ADR-005: Key-Only Dropbear + `openssh-sftp-server` for OEM Watchdog Safety
- **Context**: On ColorOS/OPLUS kernels with `oplus_secure_guard_new.ko`, OpenSSH `sshd` privilege separation drops to a non-root UID while `egid=0` and calls `setgid`/`setresgid`, triggering an immediate security reboot (`reboot,malicious_app_try_to_root_devices`).
- **Decision**: Default container init runs foreground key-only Dropbear (`dropbear -F -E -s -g -m`) paired with `openssh-sftp-server` for SFTP/SCP, and container provisioning enforces `APT::Sandbox::User "root";`.
- **Consequences**: Reliable SSH, port forwarding, and SFTP workflows with zero OEM security watchdog triggers.

## ADR-006: Standalone Control Plane Aligned with SideVeil JSON-RPC
- **Context**: Strata serves both as a standalone developer Linux runtime (usable even to debug SideVeil itself) and as a SideVeil `uds-native` plugin backend (`strata.manager`).
- **Decision**: `stratad` exposes a standalone NDJSON JSON-RPC 2.0 control plane over `/data/adb/strata/run/strata.sock` (`0700 root:root`, `SO_PEERCRED` `uid == 0`) routed through a transport-agnostic `Dispatcher` and `PluginEventSink`. SideVeil `hub.sock` connectivity is isolated behind an optional auto-reconnecting seam.
- **Consequences**: 100% standalone operation when SideVeil is absent or down, with zero protocol translation overhead when bridging to SideVeil in Phase 2.

## ADR-007: Pure Bearer HTTP/WS API + Tauri v2 Desktop Companion (No Browser-Hosted UI)
- **Context**: Full container lifecycle management (provisioning, cloning, interactive terminal, managed services) is awkward on mobile touchscreens, while serving a full web console from `stratad` would add cookie session management, CSRF surface, and dual-breakpoint CSS complexity.
- **Decision**: Expose a minimal Bearer-authenticated API in `stratad/src/web/` (`GET /api/info`, `POST /api/rpc`, `GET /ws/events`, `GET/DELETE /ws/pty/:id`, default loopback `127.0.0.1:7681` with opt-in RFC1918 LAN toggle), paired with a dedicated cross-platform Tauri v2 companion (`desktop/`) reusing `@gojona/ui` and a standalone KernelSU WebUI (`webroot/`) for on-device administration.
- **Consequences**: Minimal daemon attack surface, zero cookie/HTML serving in `stratad`, native USB ADB forwarding + low-latency xterm.js PTY on PC, and strict `http://tauri.localhost` CORS/origin verification.

## ADR-008: Root-Default In-Container User Without `USER_NS` or `sudo`
- **Context**: Because stock Android GKI kernels lack `CONFIG_USER_NS`, container UIDs share the host Android kernel UID namespace. The default first Linux non-root user (`uid=1000`) collides directly with Android `AID_SYSTEM`, while `1001/1002/2000/10000+` collide with system services and Android apps (affecting `netd` policy routing and UID-based proxy rules). Moreover, ColorOS `secureguard_new` monitors non-root `uid != 0` privilege transitions.
- **Decision**: Keep `root` (`uid=0`, working directory `/root/work`) as the canonical interactive, SSH, and Desktop PTY user, and do not bundle `sudo` by default. Software that strictly refuses to run as `root` should drop privileges locally via a dedicated service user avoiding Android's `1000`–`2000` and `10000+` UID ranges.
- **Consequences**: Zero UID collision with Android system/app networking rules and zero OEM kernel escalation kills.

## ADR-009: Non-Blocking `strata rpc` CLI for KernelSU Serialized `libsu` Shell
- **Context**: KernelSU Manager executes `ksu.exec()` commands sequentially through a single shared `libsu` root shell. If a CLI subcommand blocks reading `stdin` when invoked without explicit input, it permanently deadlocks every subsequent KernelSU WebUI page across all installed modules.
- **Decision**: `strata rpc <method> [json]` reads `stdin` only when `-` is explicitly provided as the payload argument, and KSU WebUI shell invocations redirect `</dev/null`.
- **Consequences**: Deterministic non-blocking WebUI RPCs that cannot wedge KernelSU Manager.
