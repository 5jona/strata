//! Streaming HTTP image import (`POST /api/images/import`) and zero-temp-file
//! container export (`GET /api/containers/:id/export`).

use std::fs::File;
use std::io::{self, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Deserialize;
use serde_json::json;
use strata_protocol::rpc::ContainerCreateParams;
use strata_runtime::{provision::export::write_tar_gz_directory, storage};
use strata_sys::mount;
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;

use crate::web::WebAppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ImportQuery {
    pub(super) id: String,
    pub(super) filename: String,
    #[serde(default = "default_size_mb")]
    pub(super) size_mb: u64,
    pub(super) hostname: Option<String>,
    pub(super) ssh_port: Option<u16>,
    pub(super) expected_sha256: Option<String>,
    #[serde(default)]
    pub(super) auto_start: bool,
    #[serde(default = "default_true")]
    pub(super) host_bridge: bool,
    #[serde(default = "default_true")]
    pub(super) noatime: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExportQuery {
    #[serde(default = "default_export_format")]
    pub(super) format: String,
}

fn default_size_mb() -> u64 {
    8192
}

fn default_true() -> bool {
    true
}

fn default_export_format() -> String {
    "tar.gz".to_owned()
}

/// Streams an uploaded archive or raw ext4 image directly into a newly created container.
pub(super) async fn handle_image_import(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<ImportQuery>,
    body: Body,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    if let Err((status, msg)) = state
        .auth
        .verify_request(peer.ip(), lan_enabled, &headers)
        .await
    {
        return (status, Json(json!({ "error": msg }))).into_response();
    }

    let content_len = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let params = ContainerCreateParams {
        id: q.id,
        distro_id: None,
        mirror: None,
        custom_url: None,
        source_url: None,
        expected_sha256: q.expected_sha256,
        size_mb: q.size_mb,
        hostname: q.hostname,
        ssh_port: q.ssh_port,
        ssh_public_keys: Vec::new(),
        auto_start: q.auto_start,
        host_bridge: q.host_bridge,
        noatime: q.noatime,
    };

    match state
        .jobs
        .run_streaming_import(params, q.filename, content_len, body.into_data_stream())
        .await
    {
        Ok(progress) => Json(progress).into_response(),
        Err(err) => {
            let status =
                if err.code() == strata_protocol::rpc::error_codes::CONTAINER_ALREADY_EXISTS {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::BAD_REQUEST
                };
            (
                status,
                Json(json!({ "error": err.to_string(), "code": err.code() })),
            )
                .into_response()
        }
    }
}

/// Streams a container export as `.tar.gz` or `.img.gz` with zero temporary disk files.
pub(super) async fn handle_container_export(
    State(state): State<Arc<WebAppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(container_id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let lan_enabled = state.is_lan_enabled().await;
    if let Err((status, msg)) = state
        .auth
        .verify_request(peer.ip(), lan_enabled, &headers)
        .await
    {
        return (status, Json(json!({ "error": msg }))).into_response();
    }

    let (id, cfg, _) = match state.manager.resolve_container(Some(&container_id)).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };
    if let Err(err) = state.manager.ensure_no_temp_lease(&id).await {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": err.to_string(), "code": err.code() })),
        )
            .into_response();
    }

    let format = q.format.as_str();
    if format != "tar.gz" && format != "img.gz" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "format must be 'tar.gz' or 'img.gz'" })),
        )
            .into_response();
    }

    let (tx, rx) = mpsc::channel::<io::Result<bytes::Bytes>>(8);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_bg = Arc::clone(&cancel);
    let mgr = Arc::clone(&state.manager);
    let wakelock = Arc::clone(&state.wakelock);
    let is_img_gz = format == "img.gz";
    let cid = id.clone();

    tokio::spawn(async move {
        let _wl = wakelock.acquire();
        let slot = mgr.get_or_create_slot(&cid).await;
        let _lock = slot.lifecycle_lock.lock().await;
        if let Err(err) = mgr.ensure_no_temp_lease(&cid).await {
            let _ = tx.send(Err(io::Error::other(err.to_string()))).await;
            return;
        }

        let was_running = {
            let st = slot.state.read().await;
            st.phase.is_running()
        };
        if is_img_gz
            && was_running
            && let Err(err) = mgr.stop_locked(&cid, &cfg, &slot).await
        {
            let _ = tx.send(Err(io::Error::other(err.to_string()))).await;
            return;
        }

        let mdir = mgr.module_dir.clone();
        let cfg_block = cfg.clone();
        let cid_block = cid.clone();
        let tx_block = tx.clone();

        let _ = tokio::task::spawn_blocking(move || {
            let writer = ChannelWriter {
                tx: tx_block,
                cancel: Arc::clone(&cancel_bg),
            };
            if is_img_gz {
                if let Ok(file) = File::open(&cfg_block.image_path) {
                    let mut reader = BufReader::with_capacity(256 * 1024, file);
                    let mut enc = GzEncoder::new(writer, Compression::fast());
                    let _ = io::copy(&mut reader, &mut enc);
                    let _ = enc.finish();
                }
            } else {
                let mount_dir = PathBuf::from(&cfg_block.mount_dir);
                let was_mounted = mount::is_ext4_mounted(&mount_dir).unwrap_or(false);
                if !was_mounted && storage::mount_rootfs(&cfg_block, mdir.as_deref()).is_err() {
                    return;
                }
                let _ = write_tar_gz_directory(&mount_dir, writer, &cancel_bg);
                if !was_mounted {
                    let _ = storage::teardown_container(&cid_block, &cfg_block, None, None);
                }
            }
        })
        .await;

        if is_img_gz && was_running {
            let _ = mgr.start_locked(&cid, &cfg, &slot, false).await;
        }
    });

    let stream = tokio_stream_from_rx(rx, cancel);
    let filename = format!("strata-{id}.{format}");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

struct ChannelWriter {
    tx: mpsc::Sender<io::Result<bytes::Bytes>>,
    cancel: Arc<AtomicBool>,
}

impl io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "export cancelled",
            ));
        }
        let chunk = bytes::Bytes::copy_from_slice(buf);
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client disconnected"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn tokio_stream_from_rx(
    rx: mpsc::Receiver<io::Result<bytes::Bytes>>,
    cancel: Arc<AtomicBool>,
) -> impl futures_util::Stream<Item = io::Result<bytes::Bytes>> + Send + 'static {
    let reader = tokio_util::io::StreamReader::new(RxStream { rx, cancel });
    ReaderStream::with_capacity(reader, 64 * 1024)
}

struct RxStream {
    rx: mpsc::Receiver<io::Result<bytes::Bytes>>,
    cancel: Arc<AtomicBool>,
}

impl futures_util::Stream for RxStream {
    type Item = io::Result<bytes::Bytes>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for RxStream {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
