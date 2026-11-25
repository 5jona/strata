//! Subcommand argument enums for `ui`, `gesture`, `cdp`, `app`, and `system`.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub(crate) enum UiCmd {
    Dump {
        #[arg(long)]
        compressed: bool,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        resource_id: Option<String>,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        clickable_only: bool,
        #[arg(long)]
        compact: bool,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    Tap {
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        resource_id: Option<String>,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long, default_value_t = 0)]
        index: usize,
        #[arg(long, default_value = "input")]
        method: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum GestureCmd {
    Tap {
        x: i32,
        y: i32,
    },
    LongPress {
        x: i32,
        y: i32,
        #[arg(long, default_value_t = 600)]
        duration_ms: u64,
    },
    Swipe {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        #[arg(long, default_value_t = 240)]
        duration_ms: u64,
    },
    EdgeSwipe {
        #[arg(long, default_value = "right")]
        side: String,
        #[arg(long, default_value_t = 50.0)]
        y_percent: f64,
        #[arg(long, default_value_t = 96.0)]
        distance_dp: f64,
        #[arg(long, default_value_t = 240)]
        duration_ms: u64,
        #[arg(long)]
        cancel_back: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum CdpCmd {
    ListTargets {
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
    },
    Eval {
        expression: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        url_match: Option<String>,
        #[arg(long, default_value_t = 5.0)]
        timeout: f64,
    },
    Audit {
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        url_match: Option<String>,
        #[arg(long)]
        eval_js: Option<String>,
        #[arg(long, default_value_t = 5.0)]
        timeout: f64,
    },
    Console {
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        url_match: Option<String>,
        #[arg(long, default_value_t = 2.0)]
        duration: f64,
        #[arg(long, default_value_t = 5.0)]
        timeout: f64,
    },
    Forward {
        #[arg(long, default_value_t = 9222)]
        port: u16,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AppCmd {
    Inspect {
        #[arg(default_value = "auto")]
        package: String,
        #[arg(long)]
        top_only: bool,
    },
    Control {
        action: String,
        package: String,
        #[arg(long)]
        activity: Option<String>,
        #[arg(long)]
        intent_action: Option<String>,
        #[arg(long)]
        data_uri: Option<String>,
    },
    Install {
        apk_path: String,
        #[arg(long)]
        no_replace: bool,
        #[arg(long)]
        no_grant: bool,
        #[arg(long)]
        downgrade: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SystemCmd {
    Pstore {
        #[arg(long, default_value = "/sys/fs/pstore")]
        dir: String,
    },
    Dmesg {
        #[arg(long, default_value_t = 80)]
        limit: usize,
        #[arg(long)]
        filter: Option<String>,
    },
    Tracing {
        #[arg(long, default_value = "/sys/kernel/tracing")]
        dir: String,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    Ksu {
        mode: String,
        path: Option<String>,
    },
    Lsposed {
        mode: String,
        #[arg(long)]
        module: Option<String>,
        #[arg(long)]
        grep: Option<String>,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    Sqlite {
        mode: String,
        db_path: String,
        #[arg(long)]
        sql: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    KernelProbe {
        preset: String,
        #[arg(long, default_value_t = 5)]
        duration: u64,
    },
}
