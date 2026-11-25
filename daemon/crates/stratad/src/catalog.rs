//! Simplestreams ARM64 rootfs distro catalog client (TUNA primary + LinuxContainers fallback)
//! with 30-minute bounded in-memory cache, HTTPS-only SSRF guards, and 8 MiB response caps.

mod parser;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use strata_protocol::constants::DEFAULT_DISTRO_MIRROR_OFFICIAL;
use strata_protocol::rpc::{DistroCatalogParams, DistroCatalogResult, DistroEntry, DistroMirror};
use tokio::sync::Mutex;

use crate::error::{DaemonError, DaemonResult};
pub(crate) use parser::validate_remote_https_url;
use parser::{parse_simplestreams_images, resolve_mirror_url};

const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_CACHE_ENTRIES: usize = 16;
const MAX_CATALOG_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
struct CachedCatalog {
    fetched_at: Instant,
    fetched_at_ms: u64,
    entries: Vec<DistroEntry>,
}

/// HTTP client + bounded TTL cache for ARM64 Linux rootfs Simplestreams catalogs.
#[derive(Debug)]
pub(crate) struct CatalogClient {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, CachedCatalog>>,
}

impl CatalogClient {
    /// Builds a `CatalogClient` using `rustls` WebPKI roots and HTTPS-only anti-SSRF redirects.
    pub(crate) fn new() -> Result<Self, String> {
        let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many HTTP redirects");
            }
            if let Err(err) = validate_remote_https_url(attempt.url().as_str()) {
                return attempt.error(err.to_string());
            }
            attempt.follow()
        });
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(redirect_policy)
            .user_agent("strata/0.1")
            .build()
            .map_err(|e| format!("init reqwest client: {e}"))?;
        Ok(Self {
            http,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Returns a reference to the shared HTTP client for streaming rootfs downloads.
    #[must_use]
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Fetches (or returns cached) ARM64 default-variant rootfs entries from the requested mirror.
    pub(crate) async fn fetch_catalog(
        &self,
        params: DistroCatalogParams,
    ) -> DaemonResult<DistroCatalogResult> {
        let mirror = params.mirror.unwrap_or(DistroMirror::Tuna);
        let base_url = resolve_mirror_url(mirror, params.custom_url.as_deref())?;

        if !params.refresh {
            let guard = self.cache.lock().await;
            if let Some(hit) = guard.get(&base_url)
                && hit.fetched_at.elapsed() < CACHE_TTL
            {
                return Ok(DistroCatalogResult {
                    mirror,
                    base_url,
                    cached: true,
                    fetched_at_ms: hit.fetched_at_ms,
                    entries: hit.entries.clone(),
                });
            }
        }

        let (resolved_mirror, resolved_base, entries) = match self.download_catalog(&base_url).await
        {
            Ok(list) => (mirror, base_url, list),
            Err(primary_err) if mirror == DistroMirror::Tuna => {
                let fallback = DEFAULT_DISTRO_MIRROR_OFFICIAL.to_owned();
                let list = self.download_catalog(&fallback).await.map_err(|fb_err| {
                    DaemonError::Internal(format!(
                        "catalog fetch failed on TUNA ({primary_err}) and official ({fb_err})"
                    ))
                })?;
                (DistroMirror::Official, fallback, list)
            }
            Err(err) => return Err(err),
        };

        let fetched_at_ms = now_unix_ms();
        {
            let mut guard = self.cache.lock().await;
            guard.retain(|_, v| v.fetched_at.elapsed() < CACHE_TTL);
            if guard.len() >= MAX_CACHE_ENTRIES
                && !guard.contains_key(&resolved_base)
                && let Some(oldest_key) = guard
                    .iter()
                    .min_by_key(|(_, v)| v.fetched_at)
                    .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
            guard.insert(
                resolved_base.clone(),
                CachedCatalog {
                    fetched_at: Instant::now(),
                    fetched_at_ms,
                    entries: entries.clone(),
                },
            );
        }

        Ok(DistroCatalogResult {
            mirror: resolved_mirror,
            base_url: resolved_base,
            cached: false,
            fetched_at_ms,
            entries,
        })
    }

    /// Resolves a single `distro_id` (e.g. `ubuntu:noble:arm64:default`) to its latest `DistroEntry`.
    pub(crate) async fn resolve_distro(
        &self,
        distro_id: &str,
        mirror: Option<DistroMirror>,
        custom_url: Option<String>,
    ) -> DaemonResult<DistroEntry> {
        let catalog = self
            .fetch_catalog(DistroCatalogParams {
                mirror,
                custom_url,
                refresh: false,
            })
            .await?;
        catalog
            .entries
            .into_iter()
            .find(|e| e.id == distro_id)
            .ok_or_else(|| {
                DaemonError::InvalidParams(format!(
                    "distro '{distro_id}' not found in ARM64 catalog"
                ))
            })
    }

    async fn download_catalog(&self, base_url: &str) -> DaemonResult<Vec<DistroEntry>> {
        let trimmed = base_url.trim_end_matches('/');
        let url = format!("{trimmed}/streams/v1/images.json");
        validate_remote_https_url(&url)?;
        let mut resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| DaemonError::Internal(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(DaemonError::Internal(format!(
                "GET {url} returned HTTP {}",
                resp.status()
            )));
        }
        if resp
            .content_length()
            .is_some_and(|len| len > MAX_CATALOG_BODY_BYTES as u64)
        {
            return Err(DaemonError::Internal(format!(
                "catalog response from {url} exceeds {MAX_CATALOG_BODY_BYTES} byte limit"
            )));
        }
        let mut raw = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| DaemonError::Internal(format!("read {url}: {e}")))?
        {
            if raw.len().saturating_add(chunk.len()) > MAX_CATALOG_BODY_BYTES {
                return Err(DaemonError::Internal(format!(
                    "catalog response from {url} exceeds {MAX_CATALOG_BODY_BYTES} byte limit"
                )));
            }
            raw.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&raw)
            .map_err(|e| DaemonError::Internal(format!("parse {url}: {e}")))?;
        Ok(parse_simplestreams_images(trimmed, &body))
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
