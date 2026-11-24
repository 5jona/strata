# Strata Rust Workspace Guidelines

Applies to all crates under `crates/`.

- **Workspace & Module Layout**: Inherit `[lints] workspace = true`. Never use `mod.rs` (use `foo.rs` + sibling `foo/bar.rs`). Keep every source file `<= 300` lines (hard limit `400`).
- **Pure Logic Separation**: Keep protocol schemas, config validation, capability profile bitmasks, restart backoff state machines, and `/proc`/`/sys` parsers platform-agnostic and unit-tested on the host without root privileges or live sockets.
- **Safety & Reliability**: Concentrate all `unsafe` blocks in `crates/strata-sys` (or narrowly scoped `strata-runtime` exec calls) with explicit `// SAFETY:` comments. Persist configuration and runtime files atomically (`temp -> write -> sync_all -> chmod -> rename`).
- **Rust Negative-Pattern Rules** (enforced via `Cargo.toml` `[workspace.lints.clippy]` and `clippy.toml`):
  - Never call `.unwrap()` or `.expect()` in non-test runtime code (`unwrap_used`, `expect_used`; permitted only in `#[cfg(test)]`).
  - Never hold `std::sync::Mutex` / `RwLock` guards across `.await` points (`await_holding_lock`); use `tokio::sync` locks or drop sync guards before awaiting.
  - Prefer tagged `enum` state machines over multiple correlated `Option` / `bool` fields that allow invalid combinations.
  - Default new binary-crate internals to `pub(crate)` rather than `pub` (`unreachable_pub`), and avoid unnecessary `.clone()` calls (`redundant_clone`).
- **Control Plane, Rootfs & Web/PTY Security**: Bind `/data/adb/strata/run/strata.sock` inside a `0700 root:root` directory and verify `uid == 0` via `SO_PEERCRED`. Perform host-side writes inside mounted container rootfs via `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)`. Guard concurrent `create`/`import`/`clone` with slot/port locks, echo `Sec-WebSocket-Protocol` on WS upgrades, evict the oldest detached PTY before hitting session limits, and pass launch specs over inherited pipes rather than CLI args.
- **Non-Blocking KSU CLI**: Never implicitly read `stdin` in `strata rpc` unless `-` is explicitly passed; KernelSU Manager serializes commands over a shared `libsu` shell, so any blocking stdin read deadlocks all KSU WebUIs.
- **Single-Threaded Isolation Execution**: Never run complex `unshare`, `mount`, `pivot_root`, or `setns` sequences inside a `fork()` pre-exec hook of a multi-threaded Tokio process. Re-exec a dedicated single-threaded helper mode (`stratad __init` / `stratad __exec`) or execute directly in single-threaded `strata` CLI.
- **Test-Baseline Protection SOP**: Never delete, skip, or weaken existing unit/integration assertions to make refactors pass. When adopting shared SDKs or tightening lints, preserve all behavioral coverage and add regression tests for new invariants.
- **Verification**: Run `mise run rust:check` (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`).
