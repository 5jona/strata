# Strata

Strata is a standalone, zero-GKI-modification, pure-Rust multi-container Linux runtime and KernelSU module for Android 16+ (`arm64`). It replaces shell-based `ruri` wrappers with native Linux syscall orchestration (`stratad` + `strata` CLI) and aligns its JSON-RPC 2.0 control plane for seamless SideVeil plugin integration (`strata.manager`).

## Key Capabilities

- **Pure-Rust Container Runtime**: Direct `/dev/loop-control` ioctls, SELinux xattr labeling, `ext4` mounting with concurrent-mount (`IMAGE_BUSY`) protection, progressive namespace isolation (`NEWNS`, `NEWUTS`, `NEWCGROUP`, `NEWTIME`), minimal `tmpfs /dev` (hiding `/dev/block/*` by default), `pivot_root(".", ".")`, Linux v3 capability bounding, and native `FITRIM` compaction.
- **Zero-Polling Supervision & Clean Teardown**: `pidfd_open` + Tokio `AsyncFd` monitors container init processes without polling; dual-signal teardown (cgroup v2 + mount namespace inode scan) guarantees `mounts == 0 && loops == 0` without requiring `PID_NS`.
- **Multi-Container Control Plane**: `/data/adb/strata/run/strata.sock` (`0700 root:root`, `SO_PEERCRED` `uid == 0`) serves JSON-RPC 2.0 / NDJSON for CLI management, headless streaming command/Python execution, and SideVeil plugin integration.
- **Native Interactive CLI**: `strata` (symlinked at `/data/adb/ksu/bin/strata`) supports `start`, `stop`, `restart`, `status`, `list`, `enter`, `exec`, `py`, `debug`, `dns`, `logs`, `ssh-key`, `compact`, `fsck`, `backup`, and `config`, using local single-threaded `setns` for zero-overhead interactive TTY sessions.
- **Android Host Bridge**: Syncs `/etc/resolv.conf`, `/run/strata/host.env`, POSIX `host-exec`, and the Python `strata-debug` toolkit without invoking Bionic binaries inside glibc containers.

## Workspace Layout

- `crates/strata-protocol`: Config schema, JSON-RPC wire types, state machine, capability profiles, and constants.
- `crates/strata-sys`: Audited Linux/Android syscall wrappers and `/proc`/`/sys` parsers.
- `crates/strata-runtime`: Single-threaded stage-2 init, `setns` enter, storage engine, and atomic sync.
- `crates/stratad`: UID 0 Tokio daemon, `strata.sock` control plane, and optional auto-reconnecting SideVeil `hub.sock` plugin backend (`strata.manager`) with Native HUD support.
- `crates/strata-ctl`: Multi-container CLI binary (`strata`).
- `container/`: In-container POSIX/Python bridge scripts and Ubuntu provisioning helper.
- `contracts/`: Single-source-of-truth contract spec (`strata.contract.json`), generated JSON Schema, and golden wire fixtures validated via `@gojona/contracts-gen`.
- `webui/` & `plugins/strata.manager/`: First-party SideVeil plugin frontend (React 19 + TypeScript + Tailwind v4 + `@gojona/ui` + `@gojona/bridge` + `@gojona/sideveil-sdk` + `@gojona/config`).
- `ARCHITECTURE.md`: Subsystem ownership map, runtime flows, protocol boundaries, and concurrency invariants.
- `docs/architecture-decisions.md`: Empirical Android 16+ GKI constraints and architectural decision records.
- `docs/upstream-backlog.md`: Generic upstream improvements identified for `sideveil` and `gojona-kit`.

## Standalone Build & Verification

Prerequisites:

- Rust stable (`>= 1.89`) with `rustfmt`, `clippy`, and the `aarch64-linux-android` target (for module cross-compilation)
- Android NDK `30.0.16248370` (or another NDK providing `aarch64-linux-android{37,36}-clang` under `ANDROID_HOME` / `ANDROID_NDK_HOME`)
- Node.js `>= 24` and `pnpm@12.22.0`
- Python `>= 3.12`

Install WebUI dependencies (which also installs `@gojona/contracts-gen` for contract verification) and run the quality gates:

```sh
pnpm --dir webui install --frozen-lockfile
mise run check   # Contract drift/fixtures + Rust fmt/clippy/tests + WebUI lint/typecheck/build/tests + Python checks
mise run build   # Cross-compile aarch64-linux-android release binaries
bash build.sh    # Run full quality gates and package out/strata.zip
```

Without `mise`, invoke the underlying commands directly:

```sh
webui/node_modules/.bin/pochita-contracts-gen check contracts/*.contract.json
webui/node_modules/.bin/pochita-contracts-gen validate-fixtures contracts/schemas/strata.schema.json contracts/fixtures
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
pnpm --dir webui check
python3 -m unittest discover -s tests -v
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$PWD/scripts/aarch64-android-linker.sh" bash build.sh
```

### Local Co-Development with `sideveil` (Optional)

When iterating locally against an unpublished sibling checkout of [`sideveil`](https://github.com/5jona/sideveil), create a local gitignored `.cargo/config.toml` at the repository root to override the upstream Git dependency without editing `Cargo.toml`:

```toml
[patch."https://github.com/5jona/sideveil.git"]
sideveil-protocol = { path = "../sideveil/hub/crates/sideveil-protocol" }
sideveil-plugin-sdk = { path = "../sideveil/hub/crates/sideveil-plugin-sdk" }
```

Similarly, when co-developing WebUI packages with a local `gojona-kit` checkout, set `POCHITA_KIT_ROOT=/path/to/gojona-kit` when running Vite so the dev server allows reading files from that directory.

## License

Strata is licensed under the **GNU General Public License v3.0 only** (`GPL-3.0-only`). See [LICENSE](LICENSE) for the full license text.
