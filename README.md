# Strata

Strata is a standalone, zero-GKI-modification, pure-Rust multi-container Linux runtime and KernelSU module for Android 16+ (`arm64`). It replaces shell-based `ruri` wrappers with native Linux syscall orchestration (`stratad` + `strata` CLI) and aligns its JSON-RPC 2.0 control plane for seamless SideVeil plugin (`strata.manager`), standalone KernelSU WebUI, and Tauri v2 Desktop Companion integration.

## Key Capabilities

- **Pure-Rust Container Runtime**: Direct `/dev/loop-control` ioctls, SELinux xattr labeling, `ext4` mounting with concurrent-mount (`IMAGE_BUSY`) protection, progressive namespace isolation (`NEWNS`, `NEWUTS`, `NEWCGROUP`, `NEWTIME`), minimal `tmpfs /dev` (hiding `/dev/block/*` by default), `pivot_root(".", ".")`, Linux v3 capability bounding, and native `FITRIM` compaction.
- **Zero-Polling Supervision & Clean Teardown**: `pidfd_open` + Tokio `AsyncFd` monitors container init processes without polling; dual-signal teardown (cgroup v2 + mount namespace inode scan) guarantees `mounts == 0 && loops == 0` without requiring `PID_NS`.
- **Multi-Container Control Plane**: `/data/adb/strata/run/strata.sock` (`0700 root:root`, `SO_PEERCRED` `uid == 0`) and Bearer-authenticated HTTP/WS endpoints serve JSON-RPC 2.0 for CLI management, headless streaming execution, managed services, container tool management (`mise` + `uv`), and SideVeil / Desktop UIs.
- **Native Interactive CLI**: `strata` (symlinked at `/data/adb/ksu/bin/strata`) supports `start`, `stop`, `restart`, `status`, `list`, `enter`, `exec`, `py`, `debug`, `dns`, `logs`, `ssh-key`, `compact`, `fsck`, `backup`, `tools`, `web`, and `config`, using local single-threaded `setns` for zero-overhead interactive TTY sessions.
- **Android Host Bridge & Static Rust Debug Agent**: Syncs `/etc/resolv.conf`, `/run/strata/host.env`, canonical `/etc/profile.d/strata-env.sh` + `/etc/strata/layout.json`, POSIX `host-exec`, and the static musl `strata-debug` binary without invoking Bionic binaries inside glibc containers.

## Workspace Layout

- `daemon/`: Android Rust workspace (`daemon/Cargo.toml`):
  - `daemon/crates/strata-protocol`: Config schema, JSON-RPC wire types, state machine, capability profiles, and constants.
  - `daemon/crates/strata-sys`: Audited Linux/Android syscall wrappers and `/proc`/`/sys` parsers.
  - `daemon/crates/strata-runtime`: Single-threaded stage-2 init, `setns` enter, storage engine, layout sync, and atomic sync.
  - `daemon/crates/stratad`: UID 0 Tokio daemon, `strata.sock` + HTTP/WS control plane, tool manager, and optional auto-reconnecting SideVeil `hub.sock` plugin backend (`strata.manager`).
  - `daemon/crates/strata-ctl`: Multi-container CLI binary (`strata`).
- `tools/`: Standalone Rust debugging tools:
  - `tools/strata-debug-agent`: Static musl (`aarch64-unknown-linux-musl`) on-device L1–L5 debugging binary (`strata-debug`).
  - `tools/strata-mcp`: Host-side async Rust MCP server (`strata-mcp`).
- `module/`: KernelSU module static sources (`module.prop`, `config.json`, lifecycle scripts, `container/{host-exec,setup-container.sh}`, `plugins/strata.manager/manifest.json`).
- `contracts/`: Single-source-of-truth contract spec (`strata.contract.json`), generated JSON Schema, and golden wire fixtures validated via `@gojona/contracts-gen`.
- `webui/`: React 19 + TypeScript + Tailwind v4 frontends built into `out/stage/plugins/strata.manager/web/` and `out/stage/webroot/`.
- `desktop/`: Cross-platform Tauri v2 PC companion (`desktop/src/`, `desktop/crates/strata-desktop-core/`, `desktop/src-tauri/`).
- `scripts/`: Build, hot-reload, version management (`bump-version.mjs`), device bootstrap, and live verification scripts.
- `docs/`: Architecture map (`docs/ARCHITECTURE.md`), changelog (`docs/CHANGELOG.md`), GKI decision records (`docs/architecture-decisions.md`), and upstream backlog.

## Standalone Build & Verification

Prerequisites:

- Rust stable (`>= 1.89`) with `rustfmt`, `clippy`, `aarch64-linux-android`, and `aarch64-unknown-linux-musl` targets
- Android NDK `30.0.16248370` (providing `aarch64-linux-android{37,36}-clang` under `ANDROID_HOME` / `ANDROID_NDK_HOME`)
- Node.js `>= 24` and `pnpm@12.22.0`

Install frontend dependencies and run the quality gates:

```sh
pnpm --dir webui install --frozen-lockfile
pnpm --dir desktop install --frozen-lockfile
mise run check         # version:check + contracts:check + rust:check + webui:check + desktop:check
mise run build         # Cross-compile daemon + static musl debug agent + WebUI and package out/strata.zip
mise run build:mcp     # Build host-side Rust strata-mcp release binary
```

Without `mise`, invoke the underlying commands directly:

```sh
node scripts/bump-version.mjs --check
webui/node_modules/.bin/pochita-contracts-gen check contracts/*.contract.json
webui/node_modules/.bin/pochita-contracts-gen validate-fixtures contracts/schemas/strata.schema.json contracts/fixtures
(cd daemon && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)
(cd tools/strata-debug-agent && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)
(cd tools/strata-mcp && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)
pnpm --dir webui check
pnpm --dir desktop check
bash scripts/build.sh
```

### Local Co-Development with `sideveil` & `gojona-kit` (Optional)

When iterating locally against an unpublished sibling checkout of [`sideveil`](https://github.com/5jona/sideveil), create a gitignored `daemon/.cargo/config.toml` to override the upstream Git dependency without editing `daemon/Cargo.toml`:

```toml
[patch."https://github.com/5jona/sideveil.git"]
sideveil-protocol = { path = "../../sideveil/hub/crates/sideveil-protocol" }
sideveil-plugin-sdk = { path = "../../sideveil/hub/crates/sideveil-plugin-sdk" }
```

Similarly, when co-developing WebUI or Desktop packages with a local `gojona-kit` checkout, set `POCHITA_KIT_ROOT=/path/to/gojona-kit` when running Vite or `pnpm check`.

## License

Strata is licensed under the **GNU General Public License v3.0 only** (`GPL-3.0-only`). See [LICENSE](LICENSE) for the full license text.
