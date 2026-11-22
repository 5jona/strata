//! Resilient auto-reconnecting supervisor loop targeting SideVeil's `hub.sock`.

use std::io;
use std::time::Duration;

use tokio::sync::watch;

use crate::sideveil_seam::bridge::BridgeDeps;
use crate::sideveil_seam::session::run_connected_session;
use crate::sideveil_seam::wire::connect_and_handshake;

/// Runs the resilient, non-fatal reconnect loop targeting SideVeil's `hub.sock`.
pub(super) async fn run_reconnect_loop(mut deps: BridgeDeps) {
    let mut backoff_secs: u64 = 1;
    let mut logged_missing_socket = false;

    loop {
        if *deps.shutdown.borrow() {
            return;
        }
        let cfg = deps.config_store.snapshot().await.sideveil;
        if !cfg.enabled {
            deps.seam.set_connected(false);
            tokio::select! {
                _ = wait_shutdown(&mut deps.shutdown) => return,
                _ = deps.seam.wait_config_wake() => continue,
                _ = tokio::time::sleep(Duration::from_secs(10)) => continue,
            }
        }

        match connect_and_handshake(&cfg.hub_sock, &cfg.plugin_id).await {
            Ok((reader, write_half)) => {
                backoff_secs = 1;
                logged_missing_socket = false;
                deps.seam.set_connected(true);
                eprintln!(
                    "stratad: connected to SideVeil hub '{}' as plugin '{}'",
                    cfg.hub_sock, cfg.plugin_id
                );

                let shutdown_requested = run_connected_session(
                    reader,
                    write_half,
                    &cfg.hub_sock,
                    &cfg.plugin_id,
                    &mut deps,
                )
                .await;
                deps.seam.set_connected(false);
                if shutdown_requested {
                    return;
                }
                eprintln!("stratad: SideVeil hub session closed; will reconnect automatically");
            }
            Err(err) => {
                deps.seam.set_connected(false);
                let is_absent = matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                );
                if !is_absent {
                    eprintln!("stratad: SideVeil bridge connect failed: {err}");
                } else if !logged_missing_socket {
                    eprintln!(
                        "stratad: SideVeil hub '{}' unavailable (running standalone, auto-retrying)",
                        cfg.hub_sock
                    );
                    logged_missing_socket = true;
                }
            }
        }

        tokio::select! {
            _ = wait_shutdown(&mut deps.shutdown) => return,
            _ = deps.seam.wait_config_wake() => {
                backoff_secs = 1;
            }
            _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {
                backoff_secs = (backoff_secs * 2).min(15);
            }
        }
    }
}

pub(super) async fn wait_shutdown(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}
