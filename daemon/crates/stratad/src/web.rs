//! Embedded `axum` HTTP + WebSocket control plane for `strata-desktop` (Tauri v2) and API clients.
//! Defaults to `127.0.0.1:7681` with runtime-toggleable RFC1918 LAN mode, `TCP_NODELAY`,
//! immediate session revocation on token rotation, and bounded hot-rebind drain.

pub(crate) mod auth;
mod auth_policy;
mod cors;
mod events;
mod pty;
mod pty_codec;
mod pty_io;
pub(crate) mod pty_session;
mod rpc;
#[cfg(test)]
mod tests;
mod transfer;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::middleware;
use axum::routing::{delete, get, post};
use axum::serve::ListenerExt;
use strata_protocol::WebConsoleConfig;
use strata_protocol::constants::{DEFAULT_WEB_LISTEN_LAN, DEFAULT_WEB_LISTEN_LOCAL};
use strata_protocol::rpc::{WebConfigureParams, WebRotateTokenResult, WebStatusResult};
use strata_sys::net;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, oneshot, watch};

use crate::config_store::ConfigStore;
use crate::dispatcher::Dispatcher;
use crate::error::{DaemonError, DaemonResult};
use crate::jobs::ProvisionJobManager;
use crate::manager::ContainerManager;
use crate::sideveil_seam::EventHub;
use crate::wakelock::WakeLockManager;
use auth::WebAuthState;
use pty_session::PtyRegistry;

const REBIND_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
const REBIND_WAIT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Shared state injected into `axum` route handlers.
#[derive(Debug)]
pub(crate) struct WebAppState {
    pub(crate) config_store: Arc<ConfigStore>,
    pub(crate) dispatcher: Arc<Dispatcher>,
    pub(crate) manager: Arc<ContainerManager>,
    pub(crate) event_hub: Arc<EventHub>,
    pub(crate) jobs: Arc<ProvisionJobManager>,
    pub(crate) auth: Arc<WebAuthState>,
    pub(crate) ptys: Arc<PtyRegistry>,
    pub(crate) wakelock: Arc<WakeLockManager>,
    pub(crate) session_epoch: watch::Receiver<u64>,
    pub(crate) active_events_ws: Arc<AtomicUsize>,
}

impl WebAppState {
    pub(crate) async fn is_lan_enabled(&self) -> bool {
        self.config_store.snapshot().await.web.lan_enabled
    }
}

/// Manages the lifecycle, dynamic hot-rebind, auth token, and PTY sessions of the HTTP/WS server.
#[derive(Debug)]
pub(crate) struct WebServerController {
    config_store: Arc<ConfigStore>,
    auth: Arc<WebAuthState>,
    ptys: Arc<PtyRegistry>,
    jobs: Arc<ProvisionJobManager>,
    running: Arc<AtomicBool>,
    rebind_wake: Arc<Notify>,
    rebind_lock: Mutex<()>,
    session_epoch_tx: watch::Sender<u64>,
    active_events_ws: Arc<AtomicUsize>,
    rebind_done_tx: watch::Sender<u64>,
}

impl WebServerController {
    /// Initializes token persistence and PTY registry before starting the listener loop.
    pub(crate) fn new(
        token_path: PathBuf,
        config_store: Arc<ConfigStore>,
        jobs: Arc<ProvisionJobManager>,
        wakelock: Arc<WakeLockManager>,
    ) -> Result<Self, String> {
        let auth = Arc::new(WebAuthState::load_or_generate(token_path)?);
        let ptys = Arc::new(PtyRegistry::new(wakelock));
        let (session_epoch_tx, _) = watch::channel(0u64);
        let (rebind_done_tx, _) = watch::channel(0u64);
        Ok(Self {
            config_store,
            auth,
            ptys,
            jobs,
            running: Arc::new(AtomicBool::new(false)),
            rebind_wake: Arc::new(Notify::new()),
            rebind_lock: Mutex::new(()),
            session_epoch_tx,
            active_events_ws: Arc::new(AtomicUsize::new(0)),
            rebind_done_tx,
        })
    }

    #[must_use]
    pub(crate) fn ptys(&self) -> Arc<PtyRegistry> {
        Arc::clone(&self.ptys)
    }

    /// Returns live status snapshot for `web.status` (never exposes plaintext token).
    pub(crate) async fn status(&self) -> WebStatusResult {
        let cfg = self.config_store.snapshot().await.web;
        let listen_addr = if cfg.lan_enabled {
            DEFAULT_WEB_LISTEN_LAN.to_owned()
        } else {
            DEFAULT_WEB_LISTEN_LOCAL.to_owned()
        };
        let mut lan_urls = vec![format!("http://127.0.0.1:{}", cfg.port)];
        if cfg.lan_enabled {
            let ipv4 = net::detect_primary_ipv4();
            if ipv4 != "127.0.0.1" && !ipv4.is_empty() {
                lan_urls.push(format!("http://{ipv4}:{}", cfg.port));
            }
        }
        WebStatusResult {
            enabled: cfg.enabled,
            running: self.running.load(Ordering::Acquire),
            lan_enabled: cfg.lan_enabled,
            listen_addr,
            port: cfg.port,
            token_path: self.auth.token_path().to_string_lossy().into_owned(),
            webroot_available: false,
            active_pty_sessions: self.ptys.active_count().await,
            active_jobs: self.jobs.active_count().await,
            lan_urls,
        }
    }

    /// Updates `config.json` web settings, revokes sessions if disabled/LAN-off, and waits for rebind.
    pub(crate) async fn configure(
        &self,
        params: WebConfigureParams,
    ) -> DaemonResult<WebStatusResult> {
        let _guard = self.rebind_lock.lock().await;
        let prev = self.config_store.snapshot().await.web;
        let updated: WebConsoleConfig = self.config_store.update_web_config(params).await?;

        if !updated.enabled || (prev.lan_enabled && !updated.lan_enabled) {
            self.session_epoch_tx
                .send_modify(|v| *v = v.wrapping_add(1));
        }

        let mut done_rx = self.rebind_done_tx.subscribe();
        let _ = done_rx.borrow_and_update();
        self.rebind_wake.notify_one();
        let _ = tokio::time::timeout(REBIND_WAIT_TIMEOUT, done_rx.changed()).await;

        Ok(self.status().await)
    }

    /// Rotates the 256-bit Bearer token atomically and immediately revokes all active WS/PTY sessions.
    pub(crate) async fn rotate_token(&self) -> DaemonResult<WebRotateTokenResult> {
        let (token, token_path) = self
            .auth
            .rotate_token()
            .await
            .map_err(DaemonError::Internal)?;
        let ws_revoked = self.active_events_ws.load(Ordering::Acquire);
        let ptys_revoked = self.ptys.kill_all().await;
        self.session_epoch_tx
            .send_modify(|v| *v = v.wrapping_add(1));

        Ok(WebRotateTokenResult {
            token,
            token_path,
            sessions_revoked: ws_revoked.saturating_add(ptys_revoked),
        })
    }

    /// Notifies the HTTP server loop that `config.json` changed on disk.
    pub(crate) fn notify_config_changed(&self) {
        self.rebind_wake.notify_one();
    }

    /// Spawns the auto-rebinding HTTP/WS server loop exiting cleanly when `shutdown` fires.
    pub(crate) fn spawn_server(
        self: &Arc<Self>,
        dispatcher: Arc<Dispatcher>,
        manager: Arc<ContainerManager>,
        event_hub: Arc<EventHub>,
        wakelock: Arc<WakeLockManager>,
        mut shutdown: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let state = Arc::new(WebAppState {
            config_store: Arc::clone(&self.config_store),
            dispatcher,
            manager,
            event_hub,
            jobs: Arc::clone(&self.jobs),
            auth: Arc::clone(&self.auth),
            ptys: Arc::clone(&self.ptys),
            wakelock,
            session_epoch: self.session_epoch_tx.subscribe(),
            active_events_ws: Arc::clone(&self.active_events_ws),
        });
        let running = Arc::clone(&self.running);
        let wake = Arc::clone(&self.rebind_wake);
        let done_tx = self.rebind_done_tx.clone();
        let epoch_tx = self.session_epoch_tx.clone();

        tokio::spawn(async move {
            let mut last_lan = state.config_store.snapshot().await.web.lan_enabled;
            loop {
                if *shutdown.borrow() {
                    break;
                }
                let web_cfg = state.config_store.snapshot().await.web;
                if !web_cfg.enabled || (last_lan && !web_cfg.lan_enabled) {
                    epoch_tx.send_modify(|v| *v = v.wrapping_add(1));
                }
                last_lan = web_cfg.lan_enabled;

                if !web_cfg.enabled {
                    running.store(false, Ordering::Release);
                    done_tx.send_modify(|v| *v = v.wrapping_add(1));
                    tokio::select! {
                        _ = wait_shutdown(&mut shutdown) => break,
                        _ = wake.notified() => continue,
                    }
                }

                let addr = if web_cfg.lan_enabled {
                    format!("{DEFAULT_WEB_LISTEN_LAN}:{}", web_cfg.port)
                } else {
                    format!("{DEFAULT_WEB_LISTEN_LOCAL}:{}", web_cfg.port)
                };

                let listener = match TcpListener::bind(&addr).await {
                    Ok(l) => l,
                    Err(err) => {
                        running.store(false, Ordering::Release);
                        done_tx.send_modify(|v| *v = v.wrapping_add(1));
                        eprintln!("stratad: web server failed to bind {addr}: {err}");
                        tokio::select! {
                            _ = wait_shutdown(&mut shutdown) => break,
                            _ = wake.notified() => continue,
                            _ = tokio::time::sleep(Duration::from_secs(3)) => continue,
                        }
                    }
                };

                running.store(true, Ordering::Release);
                done_tx.send_modify(|v| *v = v.wrapping_add(1));

                let router = build_router(Arc::clone(&state));
                let listener = listener.tap_io(|stream| {
                    let _ = stream.set_nodelay(true);
                });
                let (grace_tx, grace_rx) = oneshot::channel::<()>();
                let mut serve_task = tokio::spawn(async move {
                    axum::serve(
                        listener,
                        router.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .with_graceful_shutdown(async move {
                        let _ = grace_rx.await;
                    })
                    .await
                });

                let mut triggered_stop = false;
                tokio::select! {
                    _ = wait_shutdown(&mut shutdown) => {
                        triggered_stop = true;
                        epoch_tx.send_modify(|v| *v = v.wrapping_add(1));
                        let _ = grace_tx.send(());
                    }
                    _ = wake.notified() => {
                        triggered_stop = true;
                        let _ = grace_tx.send(());
                    }
                    res = &mut serve_task => {
                        if let Ok(Err(err)) = res {
                            eprintln!("stratad: web server stopped with error: {err}");
                        }
                    }
                }

                if triggered_stop
                    && tokio::time::timeout(REBIND_DRAIN_TIMEOUT, &mut serve_task)
                        .await
                        .is_err()
                {
                    serve_task.abort();
                    let _ = serve_task.await;
                }
                running.store(false, Ordering::Release);
            }
        })
    }
}

/// Builds the complete `axum` router for `strata-desktop` and API integration tests.
pub(crate) fn build_router(state: Arc<WebAppState>) -> Router {
    Router::new()
        .route("/api/info", get(rpc::handle_info))
        .route("/api/rpc", post(rpc::handle_rpc))
        .route("/ws/events", get(events::handle_events_ws))
        .route("/ws/pty/{id}", get(pty::handle_pty_ws))
        .route("/api/pty/{session_id}", delete(pty::handle_pty_delete))
        .route("/api/images/import", post(transfer::handle_image_import))
        .route(
            "/api/containers/{id}/export",
            get(transfer::handle_container_export),
        )
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            cors::cors_middleware,
        ))
        .with_state(state)
}

async fn wait_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}
