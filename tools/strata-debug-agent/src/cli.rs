//! Clap CLI schema for the `strata-debug` on-device static binary.

mod subcmds;

use clap::{Parser, Subcommand};

pub(crate) use subcmds::{AppCmd, CdpCmd, GestureCmd, SystemCmd, UiCmd};

#[derive(Debug, Parser)]
#[command(
    name = "strata-debug",
    version = env!("CARGO_PKG_VERSION"),
    about = "Pure-Rust L1-L5 Android debugging agent for Strata"
)]
pub(crate) struct Cli {
    /// Emit structured JSON output (default behavior; kept for explicit CLI parity).
    #[arg(long, global = true)]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) cmd: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Concurrent Strata daemon, container, foreground window, SurfaceFlinger, and gfxinfo probe.
    Health {
        #[arg(long, default_value = "auto")]
        package: String,
        #[arg(long)]
        no_sf: bool,
        #[arg(long)]
        no_strata: bool,
        #[arg(long)]
        include_sideveil: bool,
        #[arg(long)]
        no_sideveil: bool,
    },
    /// Raw screencap health check, ROI crop, SSIM compare, and PNG/JPEG/WebP export.
    Visual {
        #[arg(long)]
        roi: Option<String>,
        #[arg(long)]
        out: Option<String>,
        #[arg(long, default_value = "webp")]
        format: String,
        #[arg(long, default_value_t = 75)]
        quality: u8,
        #[arg(long)]
        max_width: Option<u32>,
        #[arg(long)]
        golden: Option<String>,
        #[arg(long)]
        burst_sheet: bool,
        #[arg(long, default_value_t = 80)]
        burst_interval_ms: u64,
        #[arg(long)]
        inline_base64: bool,
    },
    /// Android UI hierarchy dump (`quick-xml`) and element tapper.
    Ui {
        #[command(subcommand)]
        sub: UiCmd,
    },
    /// Native `/dev/uinput` Multitouch Protocol B gesture replay.
    Gesture {
        #[command(subcommand)]
        sub: GestureCmd,
    },
    /// Incremental logcat cursor watermark and crash/ANR highlighter.
    Logcat {
        #[arg(long)]
        mark: bool,
        #[arg(long, default_value = "/tmp/strata-debug-logcat.cursor")]
        cursor: String,
        #[arg(long, default_value = "W")]
        priority: String,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        no_advance: bool,
    },
    /// L1 WebView CDP discovery, JS eval, DOM audit, console capture, and UDS<->TCP proxy.
    Cdp {
        #[command(subcommand)]
        sub: CdpCmd,
    },
    /// Atomic L4 observe: mark -> action -> settle -> concurrent logcat/health/CDP/visual.
    Observe {
        #[arg(long, num_args = 2, value_names = ["METHOD", "JSON"])]
        action_rpc: Option<Vec<String>>,
        #[arg(long)]
        action_shell: Option<String>,
        #[arg(long)]
        action_cdp_eval: Option<String>,
        #[arg(long)]
        action_gesture: Option<String>,
        #[arg(long)]
        actions: Option<String>,
        #[arg(long)]
        wait_for_text: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 350)]
        settle_ms: u64,
        #[arg(long, default_value = "auto")]
        package: String,
        #[arg(long, default_value = "W")]
        priority: String,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        no_cdp: bool,
        #[arg(long)]
        cdp_pid: Option<u32>,
        #[arg(long)]
        visual: bool,
        #[arg(long)]
        roi: Option<String>,
        #[arg(long)]
        visual_out: Option<String>,
        #[arg(long)]
        inline_image: bool,
        #[arg(long)]
        max_width: Option<u32>,
    },
    /// Android app inspection, lifecycle control, and APK installation.
    App {
        #[command(subcommand)]
        sub: AppCmd,
    },
    /// L5 read-only diagnostics: pstore, dmesg, tracing, ksu, lsposed, sqlite, kernel-probe.
    System {
        #[command(subcommand)]
        sub: SystemCmd,
    },
    /// Perfetto capture, ring-buffer session management, and lightweight trace info.
    Perfetto {
        #[arg(long, default_value = "capture")]
        action: String,
        #[arg(long, default_value = "ui-jank")]
        preset: String,
        #[arg(long, default_value_t = 3)]
        duration: u32,
        #[arg(long)]
        buffer_kb: Option<u32>,
        #[arg(long, default_value_t = 1800)]
        max_duration: u32,
        #[arg(long, default_value = "strata_ring")]
        session: String,
        #[arg(long)]
        out: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    /// Execute a command in host (nsenter PID 1) or container scope.
    Exec {
        #[arg(long, default_value = "host")]
        scope: String,
        #[arg(long, default_value_t = 30.0)]
        timeout: f64,
        command: String,
    },
    /// Persistent NDJSON JSON-RPC 2.0 stdin/stdout server for zero-cold-start MCP sessions.
    ServeRpc,
}
