//! Entry point for `strata-debug` static Linux/Android binary.

mod app;
mod cdp;
mod cli;
mod dispatch;
mod gesture;
mod health;
mod host;
mod logcat;
mod observe;
mod perfetto;
mod rpc_loop;
mod system;
mod ui;
mod visual;

use clap::Parser;
use serde_json::{Value, json};

use cli::{CdpCmd, Cli, Command};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let parsed = Cli::parse();
    if matches!(parsed.cmd, Command::ServeRpc) {
        std::process::exit(rpc_loop::run_serve_rpc().await);
    }
    if let Command::Cdp {
        sub: CdpCmd::Forward { port, target, pid },
    } = parsed.cmd
    {
        let Some(sock) = cdp::match_socket(target.as_deref(), pid) else {
            println!(
                "{}",
                json!({"ok": false, "error": "no devtools socket found"})
            );
            std::process::exit(1);
        };
        if let Err(err) = cdp::forward::serve_cdp_proxy(port, sock.socket).await {
            println!("{}", json!({"ok": false, "error": err}));
            std::process::exit(1);
        }
        return;
    }

    let payload = dispatch::dispatch(parsed.cmd).await;
    println!("{payload}");
    if payload.get("ok").and_then(Value::as_bool) == Some(false) {
        std::process::exit(1);
    }
}
