# Strata Rust Workspace Guidelines

Applies to all crates under `crates/`.

- **Workspace & Module Layout**: Inherit `[lints] workspace = true`. Never use `mod.rs` (use `foo.rs` + sibling `foo/bar.rs`). Keep every source file `<= 300` lines (hard limit `400`).
- **Pure Logic Separation**: Keep protocol schemas, config validation, capability profile bitmasks, restart backoff state machines, and `/proc`/`/sys` parsers platform-agnostic and unit-tested on the host without root privileges or live sockets.
- **Safety & Reliability**: Concentrate all `unsafe` blocks in `crates/strata-sys` (or narrowly scoped `strata-runtime` exec calls) with explicit `// SAFETY:` comments. Never call `.unwrap()` or `.expect()` in runtime code paths (tests excluded). Persist configuration and runtime files atomically (`temp -> write -> sync_all -> chmod -> rename`).
- **Control Plane Security**: Bind `/data/adb/strata/run/strata.sock` inside a `0700 root:root` directory and verify `uid == 0` via kernel `SO_PEERCRED` on every accepted connection. Pass sensitive launch specs (`Stage2Spec` / `EnterSpec`) over inherited pipes rather than command-line arguments.
- **Single-Threaded Isolation Execution**: Never run complex `unshare`, `mount`, `pivot_root`, or `setns` sequences inside a `fork()` pre-exec hook of a multi-threaded Tokio process. Re-exec a dedicated single-threaded helper mode (`stratad __init` / `stratad __exec`) or execute directly in single-threaded `strata` CLI.
- **Verification**: Run `mise run rust:check` (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`).
