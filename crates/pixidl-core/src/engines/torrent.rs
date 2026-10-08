//! BitTorrent engine built on librqbit (a mature Rust BitTorrent library).
//!
//! - `.torrent` files and magnet links (metadata fetched via DHT/trackers)
//! - real stats from the session: progress, down/up speed, ETA, live peers
//!   (librqbit does not distinguish seeds from peers, so seeds are never shown
//!   rather than invented)
//! - file selection, per-download save folder, pause/resume/cancel
//! - the `.torrent` bytes are stored in the database after metadata is
//!   resolved, so a restart never needs to re-resolve a magnet; existing data
//!   is re-verified by piece hash when a torrent is resumed

use std::collections::HashMap;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, ManagedTorrent, Magnet, Session, SessionOptions,
    SessionPersistenceConfig, TorrentStatsState,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use ts_rs::TS;

use super::{Engine, EngineOutcome, EngineProgress, JobContext, MetaUpdate};
use crate::error::{DownloadError, Result};
use crate::security;
use crate::settings::Settings;
use crate::types::{Download, DownloadStatus, EngineKind, ErrorKind};

type Handle = Arc<ManagedTorrent>;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TorrentFile {
    pub index: u32,
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TorrentInfo {
    pub name: String,
    pub info_hash: String,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub files: Vec<TorrentFile>,
}

#[derive(Debug, Clone)]
pub enum TorrentSource {
    Magnet(String),
    Url(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TorrentEngineStatus {
    pub running: bool,
    pub listen_port: Option<u16>,
    pub dht_enabled: bool,
    pub torrents: u32,
    pub library: String,
}

pub struct TorrentEngine {
    state_dir: PathBuf,
    session: Mutex<Option<Arc<Session>>>,
    dht_enabled: std::sync::atomic::AtomicBool,
}

struct Resolved {
    bytes: Vec<u8>,
    info: TorrentInfo,
    multi_file: bool,
    trackers: Vec<String>,
    peers: Vec<SocketAddr>,
}

impl TorrentEngine {
    pub fn new(state_dir: PathBuf) -> Self {
        Self { state_dir, session: Mutex::new(None), dht_enabled: Default::default() }
    }

    /// Lazily starts the session so the app opens no ports unless torrents are used.
    async fn session(&self, settings: &Settings) -> Result<Arc<Session>> {
        let mut guard = self.session.lock().await;
        if let Some(s) = guard.as_ref() {
            return Ok(s.clone());
        }
        std::fs::create_dir_all(&self.state_dir).map_err(|e| DownloadError::fs("Cannot create torrent state folder", &e))?;
        let default_dir = PathBuf::from(&settings.default_download_dir);
        // Try dual-stack first; fall back to IPv4-only on systems with IPv6 disabled.
        let mut last_err = None;
        let mut started = None;
        for ipv4_only in [false, true] {
            let opts = self.session_options(settings, ipv4_only);
            match Session::new_with_opts(default_dir.clone(), opts).await {
                Ok(s) => {
                    started = Some(s);
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), ipv4_only, "torrent session failed to start");
                    last_err = Some(e);
                }
            }
        }
        let session = started.ok_or_else(|| {
            DownloadError::new(ErrorKind::EngineUnavailable, "Torrent engine failed to start")
                .with_detail(last_err.map(|e| format!("{e:#}")).unwrap_or_default())
        })?;
        // Torrents restored from persistence must not start on their own:
        // the queue decides what runs.
        let restored: Vec<Handle> = session.with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        for h in restored {
            if !h.stats().finished || !settings.torrent_seed_after_completion {
                let _ = session.pause(&h).await;
            }
        }
        *guard = Some(session.clone());
        Ok(session)
    }

    fn session_options(&self, settings: &Settings, ipv4_only: bool) -> SessionOptions {
        let listen_addr: SocketAddr = if ipv4_only {
            (std::net::Ipv4Addr::UNSPECIFIED, settings.torrent_listen_port).into()
        } else {
            (std::net::Ipv6Addr::UNSPECIFIED, settings.torrent_listen_port).into()
        };
        let mut opts = SessionOptions {
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json { folder: Some(self.state_dir.join("session")) }),
            listen: Some(ListenerOptions { listen_addr, ipv4_only, ..Default::default() }),
            ipv4_only,
            client_name_and_version: Some(format!("pixidl {}", env!("CARGO_PKG_VERSION"))),
            ..Default::default()
        };
        if !settings.torrent_enable_dht {
            opts.dht = None;
        }
        self.dht_enabled.store(settings.torrent_enable_dht, std::sync::atomic::Ordering::Relaxed);
        opts.ratelimits.download_bps = settings.global_speed_limit_bps.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32));
        opts.ratelimits.upload_bps = settings.global_upload_limit_bps.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32));
        opts
    }

    pub async fn status(&self) -> TorrentEngineStatus {
        let guard = self.session.lock().await;
        match guard.as_ref() {
            None => TorrentEngineStatus { running: false, listen_port: None, dht_enabled: false, torrents: 0, library: "librqbit 9".into() },
            Some(s) => TorrentEngineStatus {
                running: true,
                listen_port: s.listen_addr().map(|a| a.port()),
                dht_enabled: s.get_dht().is_some(),
                torrents: s.with_torrents(|it| it.count() as u32),
                library: "librqbit 9".into(),
            },
        }
    }

    pub async fn set_global_limits(&self, down: Option<u64>, up: Option<u64>) {
        if let Some(s) = self.session.lock().await.as_ref() {
            s.ratelimits.set_download_bps(down.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32)));
            s.ratelimits.set_upload_bps(up.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32)));
        }
    }

    pub async fn shutdown(&self) {
        if let Some(s) = self.session.lock().await.take() {
            s.stop().await;
        }
    }

    /// Resolves metadata (for magnets this may take a while: DHT/trackers).
    async fn resolve(&self, session: &Arc<Session>, source: TorrentSource) -> Result<Resolved> {
        let (add, trackers) = match &source {
            TorrentSource::Magnet(m) => {
                let parsed = Magnet::parse(m).map_err(|e| DownloadError::invalid_url("Invalid magnet link").with_detail(e.to_string()))?;
                (AddTorrent::from_url(m.clone()), parsed.trackers)
            }
            TorrentSource::Url(u) => (AddTorrent::from_url(u.clone()), vec![]),
            TorrentSource::Bytes(b) => (AddTorrent::from_bytes(b.clone()), vec![]),
        };
        let resp = session
            .add_torrent(add, Some(AddTorrentOptions { list_only: true, ..Default::default() }))
            .await
            .map_err(|e| {
                DownloadError::new(ErrorKind::TorrentMetadataUnavailable, "Torrent metadata unavailable").with_detail(format!("{e:#}"))
            })?;
        let AddTorrentResponse::ListOnly(l) = resp else {
            return Err(DownloadError::new(ErrorKind::TorrentMetadataUnavailable, "Torrent metadata unavailable"));
        };
        let name = l.info.name().map(|n| n.to_string()).unwrap_or_else(|| l.info_hash.as_string());
        let files: Vec<TorrentFile> = l
            .info
            .iter_file_details()
            .enumerate()
            .filter(|(_, f)| !f.attrs().padding)
            .map(|(i, f)| TorrentFile {
                index: i as u32,
                path: { let parts = f.filename.to_vec(); if parts.is_empty() { format!("file-{i}") } else { parts.join("/") } },
                size: f.len,
            })
            .collect();
        let multi_file = l.info.info().length.is_none();
        Ok(Resolved {
            bytes: l.torrent_bytes.to_vec(),
            info: TorrentInfo { name, info_hash: l.info_hash.as_string(), total_bytes: files.iter().map(|f| f.size).sum(), files },
            multi_file,
            trackers,
            peers: l.seen_peers,
        })
    }

    /// Metadata for the Add dialog (file list, size). Bounded by `timeout`.
    pub async fn inspect(&self, source: TorrentSource, settings: &Settings, timeout: Duration) -> Result<TorrentInfo> {
        let session = self.session(settings).await?;
        tokio::time::timeout(timeout, self.resolve(&session, source))
            .await
            .map_err(|_| {
                DownloadError::new(ErrorKind::TorrentMetadataUnavailable, "Torrent metadata not available yet")
                    .with_detail("No peers responded in time. The download can still be added; metadata will be fetched when it starts.")
            })?
            .map(|r| r.info)
    }

    fn find(session: &Arc<Session>, info_hash: &str) -> Option<Handle> {
        session.with_torrents(|it| it.map(|(_, h)| h.clone()).find(|h| h.info_hash().as_string() == info_hash))
    }
}

/// Initial display name for a magnet (its `dn`, else a short hash).
pub fn magnet_display_name(m: &str) -> Option<String> {
    let parsed = Magnet::parse(m).ok()?;
    parsed
        .name
        .clone()
        .map(|n| security::sanitize_filename(&n))
        .or_else(|| parsed.as_id20().map(|h| format!("magnet-{}", &h.as_string()[..12])))
}

/// librqbit exposes the remaining time only through its serialised form.
fn eta_secs(stats: &librqbit::TorrentStats) -> Option<u64> {
    let v = serde_json::to_value(stats).ok()?;
    v.get("live")?.get("time_remaining")?.get("duration")?.get("secs")?.as_u64()
}

impl Engine for Arc<TorrentEngine> {
    fn kind(&self) -> EngineKind {
        EngineKind::Torrent
    }

    fn run(&self, ctx: JobContext) -> BoxFuture<'static, Result<EngineOutcome>> {
        let me = self.clone();
        Box::pin(async move { me.run_job(ctx).await })
    }

    fn cleanup(&self, d: &Download, delete_completed: bool) -> BoxFuture<'static, ()> {
        let me = self.clone();
        let hash = d.info_hash.clone();
        let completed = d.status == DownloadStatus::Completed;
        let full = d.full_path();
        Box::pin(async move {
            let session = me.session.lock().await.clone();
            let delete_files = !completed || delete_completed;
            if let (Some(s), Some(h)) = (session, hash) {
                if let Some(handle) = TorrentEngine::find(&s, &h) {
                    let _ = s.delete(handle.id().into(), delete_files).await;
                    return;
                }
            }
            if delete_files && full.exists() {
                if full.is_dir() {
                    let _ = tokio::fs::remove_dir_all(&full).await;
                } else {
                    let _ = tokio::fs::remove_file(&full).await;
                }
            }
        })
    }
}

impl TorrentEngine {
    async fn run_job(&self, ctx: JobContext) -> Result<EngineOutcome> {
        let d = ctx.download.clone();
        let session = self.session(&ctx.settings).await?;
        let save_dir = PathBuf::from(&d.save_dir);
        tokio::fs::create_dir_all(&save_dir).await.map_err(|e| DownloadError::fs("Cannot create destination folder", &e))?;

        // 1. Metadata: from the stored .torrent, or resolve the magnet/URL now.
        let stored = ctx.db.torrent_data(&d.id)?;
        let source = match stored {
            Some(b) => TorrentSource::Bytes(b),
            None if d.url.starts_with("magnet:") => TorrentSource::Magnet(d.url.clone()),
            None => TorrentSource::Url(d.url.clone()),
        };
        let mut control = ctx.control.clone();
        let resolved = tokio::select! {
            biased;
            _ = JobContext::stopped(&mut control) => return Ok(EngineOutcome::Stopped),
            r = self.resolve(&session, source) => r?,
        };
        ctx.db.set_torrent_data(&d.id, &resolved.bytes)?;
        let name = security::sanitize_filename(&resolved.info.name);
        let output_folder = if resolved.multi_file { save_dir.join(&name) } else { save_dir.clone() };
        let only_files = d.engine_options.torrent_files.clone().filter(|f| !f.is_empty());
        let selected_total = match &only_files {
            Some(sel) => resolved.info.files.iter().filter(|f| sel.contains(&(f.index as usize))).map(|f| f.size).sum(),
            None => resolved.info.total_bytes,
        };
        ctx.send_meta(MetaUpdate {
            filename: Some(name.clone()),
            total_bytes: Some(selected_total),
            info_hash: Some(resolved.info.info_hash.clone()),
            title: Some(resolved.info.name.clone()),
            resumable: Some(true),
            ..Default::default()
        });

        // Disk space for what is left to fetch.
        if let Some(free) = super::available_space(&save_dir) {
            let need = selected_total.saturating_sub(d.downloaded_bytes);
            if free < need {
                return Err(DownloadError::new(ErrorKind::DiskFull, "Not enough disk space").with_detail(format!("Need {need} bytes, {free} available")));
            }
        }

        // 2. Add (or find) the torrent and make sure it is running.
        let handle = match TorrentEngine::find(&session, &resolved.info.info_hash) {
            Some(h) => h,
            None => {
                let mut opts = AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(output_folder.to_string_lossy().to_string()),
                    only_files: only_files.clone(),
                    trackers: (!resolved.trackers.is_empty()).then(|| resolved.trackers.clone()),
                    initial_peers: (!resolved.peers.is_empty()).then(|| resolved.peers.clone()),
                    ..Default::default()
                };
                opts.ratelimits.download_bps = d.speed_limit_bps.and_then(|v| NonZeroU32::new(v.min(u32::MAX as u64) as u32));
                let resp = session
                    .add_torrent(AddTorrent::from_bytes(resolved.bytes.clone()), Some(opts))
                    .await
                    .map_err(|e| DownloadError::new(ErrorKind::EngineUnavailable, "Torrent could not be started").with_detail(format!("{e:#}")))?;
                resp.into_handle().ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Torrent could not be started"))?
            }
        };
        if let Some(sel) = &only_files {
            let set: std::collections::HashSet<usize> = sel.iter().copied().collect();
            if handle.only_files().map(|o| o.into_iter().collect::<std::collections::HashSet<_>>()) != Some(set.clone()) {
                let _ = session.update_only_files(&handle, &set).await;
            }
        }
        if matches!(handle.stats().state, TorrentStatsState::Paused) {
            session
                .unpause(&handle)
                .await
                .map_err(|e| DownloadError::new(ErrorKind::EngineUnavailable, "Torrent could not be resumed").with_detail(format!("{e:#}")))?;
        }

        // 3. Poll real stats until finished / stopped / error.
        let mut announced = false;
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            let stop = tokio::select! {
                biased;
                c = JobContext::stopped(&mut control) => Some(c),
                _ = tick.tick() => None,
            };
            if let Some(c) = stop {
                match c {
                    super::Control::Cancel => {
                        let _ = session.delete(handle.id().into(), false).await;
                    }
                    _ => {
                        let _ = session.pause(&handle).await;
                    }
                }
                return Ok(EngineOutcome::Stopped);
            }
            let st = handle.stats();
            if matches!(st.state, TorrentStatsState::Error) {
                let msg = st.error.clone().unwrap_or_else(|| "unknown error".into());
                let lower = msg.to_ascii_lowercase();
                let kind = if lower.contains("no space") || lower.contains("disk full") {
                    ErrorKind::DiskFull
                } else if lower.contains("permission") || lower.contains("access is denied") {
                    ErrorKind::PermissionDenied
                } else {
                    ErrorKind::Unknown
                };
                let _ = session.pause(&handle).await;
                return Err(DownloadError::new(kind, "Torrent error").with_detail(msg));
            }
            let live = st.live.as_ref();
            if live.is_some() && !announced {
                announced = true;
                ctx.send_meta(MetaUpdate { downloading: true, ..Default::default() });
            }
            ctx.progress.set(EngineProgress {
                downloaded: st.progress_bytes,
                total: Some(st.total_bytes),
                speed_bps: live.map(|l| l.download_speed.as_bytes()).unwrap_or(0),
                upload_bps: live.map(|l| l.upload_speed.as_bytes()).unwrap_or(0),
                eta_seconds: eta_secs(&st),
                peers: live.map(|l| l.snapshot.peer_stats.live),
                seeds: None,
            });
            if st.finished {
                if !ctx.settings.torrent_seed_after_completion {
                    // Stop sharing and forget the torrent; files stay on disk.
                    let _ = session.delete(handle.id().into(), false).await;
                }
                ctx.progress.set(EngineProgress { downloaded: st.total_bytes, total: Some(st.total_bytes), ..Default::default() });
                return Ok(EngineOutcome::Completed { filename: name });
            }
        }
    }
}

/// Summary of torrents in the session, keyed by info hash, for seeding stats.
pub async fn seeding_stats(engine: &TorrentEngine) -> HashMap<String, u64> {
    let guard = engine.session.lock().await;
    let Some(s) = guard.as_ref() else { return HashMap::new() };
    s.with_torrents(|it| {
        it.filter_map(|(_, h)| {
            let st = h.stats();
            st.finished.then(|| (h.info_hash().as_string(), st.live.map(|l| l.upload_speed.as_bytes()).unwrap_or(0)))
        })
        .collect()
    })
}

