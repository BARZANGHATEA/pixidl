//! The Download Manager: queue, concurrency, priorities, scheduling, state
//! transitions, persistence, retries, engine selection, speed limits, events
//! and crash recovery.
//!
//! Every status change goes through [`Inner::transition`], which rejects
//! transitions the state machine does not allow.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{mpsc, watch, Notify};

use crate::db::{now, Category, Db, DownloadEvent};
use crate::detector::{self, InspectionError, UrlInspection};
use crate::engines::http::{self, HttpEngine};
use crate::engines::torrent::{self, TorrentEngine, TorrentEngineStatus, TorrentInfo, TorrentSource};
use crate::engines::video::{VideoEngine, VideoInfo};
use crate::engines::{Control, Engine, EngineOutcome, JobContext, MetaUpdate, ProgressCell};
use crate::error::{redact_url, DownloadError, Result};
use crate::ratelimit::{RateLimiter, RetryPolicy};
use crate::security;
use crate::settings::Settings;
use crate::tools::{self, ToolLocator, ToolStatus};
use crate::types::*;

/// Receives manager events (the Tauri shell forwards them to the UI).
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: ManagerEvent);
}

impl<F: Fn(ManagerEvent) + Send + Sync + 'static> EventSink for F {
    fn emit(&self, event: ManagerEvent) {
        self(event)
    }
}

pub struct ManagerConfig {
    pub data_dir: PathBuf,
    /// Extra directories searched for yt-dlp / FFmpeg.
    pub tool_dirs: Vec<PathBuf>,
}

/// Where an add request came from (browser requests are more restricted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddSource {
    User,
    Browser,
}

/// What the manager wants once a running job has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Pause,
    Cancel,
    Requeue,
    Remove { delete_files: bool },
}

struct Job {
    control: watch::Sender<Control>,
    progress: Arc<ProgressCell>,
    limiter: Arc<RateLimiter>,
    download: Download,
    intent: Option<Intent>,
    last_progress: crate::engines::EngineProgress,
}

struct Inner {
    db: Db,
    settings: RwLock<Arc<Settings>>,
    client: RwLock<reqwest::Client>,
    global_limiter: Arc<RateLimiter>,
    http: Arc<HttpEngine>,
    torrent: Arc<TorrentEngine>,
    video: Arc<VideoEngine>,
    tools: Arc<ToolLocator>,
    sink: Arc<dyn EventSink>,
    jobs: Mutex<HashMap<String, Job>>,
    retry_at: Mutex<HashMap<String, Instant>>,
    wake: Arc<Notify>,
    shutting_down: std::sync::atomic::AtomicBool,
    had_activity: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

impl DownloadManager {
    /// Opens the database, recovers interrupted downloads and starts the queue.
    pub async fn start(config: ManagerConfig, sink: Arc<dyn EventSink>) -> Result<Self> {
        let db = Db::open(&config.data_dir.join("nexa.db"))?;
        Self::start_with_db(db, config, sink).await
    }

    pub async fn start_with_db(db: Db, config: ManagerConfig, sink: Arc<dyn EventSink>) -> Result<Self> {
        let settings = db.load_settings()?;
        let client = http::build_client(&settings)?;
        let tools = Arc::new(ToolLocator::new(config.tool_dirs.clone()));
        let inner = Arc::new(Inner {
            global_limiter: Arc::new(RateLimiter::new(settings.global_speed_limit_bps)),
            settings: RwLock::new(Arc::new(settings)),
            client: RwLock::new(client),
            http: Arc::new(HttpEngine),
            torrent: Arc::new(TorrentEngine::new(config.data_dir.join("torrent"))),
            video: Arc::new(VideoEngine::new(tools.clone())),
            tools,
            sink,
            jobs: Mutex::new(HashMap::new()),
            retry_at: Mutex::new(HashMap::new()),
            wake: Arc::new(Notify::new()),
            shutting_down: Default::default(),
            had_activity: Default::default(),
            db,
        });
        inner.recover()?;
        let mgr = Self { inner };
        mgr.spawn_loops();
        Ok(mgr)
    }

    fn spawn_loops(&self) {
        // Queue / scheduler loop.
        let weak = Arc::downgrade(&self.inner);
        let wake = self.inner.wake.clone();
        tokio::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { break };
                if inner.shutting_down.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                inner.schedule_pass();
                drop(inner);
                let notified = wake.notified();
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        });
        // Progress sampling loop: throttles UI events to ~2/s and DB writes to every 3 s.
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                n += 1;
                inner.sample_progress(n.is_multiple_of(6));
            }
        });
    }

    pub fn db(&self) -> &Db {
        &self.inner.db
    }

    pub fn settings(&self) -> Arc<Settings> {
        self.inner.settings.read().clone()
    }

    pub fn tools(&self) -> Arc<ToolLocator> {
        self.inner.tools.clone()
    }

    // ------------------------------------------------------------------ queries

    pub fn list(&self) -> Result<Vec<Download>> {
        let mut list = self.inner.db.list_downloads()?;
        let jobs = self.inner.jobs.lock();
        for d in list.iter_mut() {
            if let Some(j) = jobs.get(&d.id) {
                *d = j.download.clone();
            }
        }
        Ok(list)
    }

    pub fn get(&self, id: &str) -> Result<Option<Download>> {
        if let Some(j) = self.inner.jobs.lock().get(id) {
            return Ok(Some(j.download.clone()));
        }
        self.inner.db.get_download(id)
    }

    pub fn events(&self, id: &str) -> Result<Vec<DownloadEvent>> {
        self.inner.db.events(id)
    }

    pub fn stats(&self) -> Result<GlobalStats> {
        let list = self.list()?;
        let mut s = GlobalStats { total: list.len() as u32, ..Default::default() };
        for d in &list {
            match d.status {
                DownloadStatus::Preparing | DownloadStatus::Downloading => {
                    s.active += 1;
                    s.download_bps += d.speed_bps;
                    s.upload_bps += d.upload_bps;
                }
                DownloadStatus::Queued => s.queued += 1,
                DownloadStatus::Completed => s.completed += 1,
                DownloadStatus::Failed => s.failed += 1,
                _ => {}
            }
        }
        Ok(s)
    }

    // ------------------------------------------------------------------ adding

    pub async fn add(&self, req: AddDownloadRequest, source: AddSource) -> Result<Download> {
        self.inner.add(req, source).await
    }

    // ------------------------------------------------------------------ control

    pub fn pause(&self, id: &str) -> Result<()> {
        self.inner.pause(id, Intent::Pause)
    }

    pub fn resume(&self, id: &str) -> Result<()> {
        let inner = &self.inner;
        let d = inner.require(id)?;
        match d.status {
            DownloadStatus::Paused => {
                inner.transition(id, DownloadStatus::Queued, |d| {
                    d.error_kind = None;
                    d.error_message = None;
                    d.error_detail = None;
                })?;
                inner.db.add_event(id, "resumed", None)?;
                inner.wake.notify_one();
                Ok(())
            }
            DownloadStatus::Failed | DownloadStatus::Cancelled => self.retry(id),
            _ => Ok(()),
        }
    }

    /// Manual retry: resets the automatic retry counter.
    pub fn retry(&self, id: &str) -> Result<()> {
        let inner = &self.inner;
        let d = inner.require(id)?;
        if !matches!(d.status, DownloadStatus::Failed | DownloadStatus::Cancelled | DownloadStatus::Paused) {
            return Ok(());
        }
        inner.retry_at.lock().remove(id);
        inner.transition(id, DownloadStatus::Queued, |d| {
            d.error_kind = None;
            d.error_message = None;
            d.error_detail = None;
            d.retry_count = 0;
            d.file_missing = false;
        })?;
        inner.db.add_event(id, "retry", Some("Retry requested"))?;
        inner.wake.notify_one();
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<()> {
        let inner = &self.inner;
        let d = inner.require(id)?;
        if d.status.is_running() {
            return inner.pause(id, Intent::Cancel);
        }
        if matches!(d.status, DownloadStatus::Completed | DownloadStatus::Cancelled) {
            return Ok(());
        }
        inner.engine(d.engine).cleanup(&d, false).await;
        inner.db.clear_segments(id)?;
        inner.retry_at.lock().remove(id);
        inner.transition(id, DownloadStatus::Cancelled, |d| {
            d.downloaded_bytes = 0;
            d.speed_bps = 0;
            d.eta_seconds = None;
        })?;
        inner.db.add_event(id, "cancelled", None)?;
        Ok(())
    }

    /// Removes the entry. Partial data is always removed; a completed file is
    /// only deleted when `delete_files` is true.
    pub async fn remove(&self, id: &str, delete_files: bool) -> Result<()> {
        let inner = &self.inner;
        let d = inner.require(id)?;
        if d.status.is_running() {
            return inner.pause(id, Intent::Remove { delete_files });
        }
        inner.finish_remove(&d, delete_files).await
    }

    pub fn pause_all(&self) -> Result<()> {
        for d in self.list()? {
            if matches!(d.status, DownloadStatus::Queued | DownloadStatus::Preparing | DownloadStatus::Downloading) {
                let _ = self.pause(&d.id);
            }
        }
        Ok(())
    }

    pub fn resume_all(&self) -> Result<()> {
        for d in self.list()? {
            if d.status == DownloadStatus::Paused {
                let _ = self.resume(&d.id);
            }
        }
        Ok(())
    }

    /// Sets the queue order. `ids` are the queued downloads in desired order.
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        self.inner.db.set_queue_positions(ids)?;
        for id in ids {
            self.inner.emit_updated(id);
        }
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn set_priority(&self, id: &str, p: Priority) -> Result<()> {
        self.inner.modify(id, |d| d.priority = p)?;
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn set_category(&self, id: &str, category: &str) -> Result<()> {
        let cats = self.inner.db.categories()?;
        if !cats.iter().any(|c| c.name == category) {
            return Err(DownloadError::new(ErrorKind::Unknown, "Unknown category"));
        }
        self.inner.modify(id, |d| d.category = category.to_string())
    }

    /// Per-download limit in bytes/s (`None` = unlimited). Applies live to HTTP;
    /// video/torrent engines apply it when the download (re)starts.
    pub fn set_download_limit(&self, id: &str, limit: Option<u64>) -> Result<()> {
        let limit = limit.filter(|v| *v > 0);
        if let Some(j) = self.inner.jobs.lock().get(id) {
            j.limiter.set_rate(limit);
        }
        self.inner.modify(id, |d| d.speed_limit_bps = limit)
    }

    /// Schedules a single download to start no earlier than `at` (RFC 3339), or clears it.
    pub fn schedule(&self, id: &str, at: Option<String>) -> Result<()> {
        if let Some(a) = &at {
            chrono::DateTime::parse_from_rfc3339(a).map_err(|e| DownloadError::new(ErrorKind::Unknown, "Invalid date").with_detail(e.to_string()))?;
        }
        self.inner.modify(id, |d| d.scheduled_at = at.clone())?;
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn set_global_limit(&self, limit: Option<u64>) -> Result<()> {
        let mut s = (*self.settings()).clone();
        s.global_speed_limit_bps = limit.filter(|v| *v > 0);
        self.update_settings_sync(s).map(|_| ())
    }

    pub fn clear_history(&self) -> Result<()> {
        for id in self.inner.db.clear_history()? {
            self.inner.sink.emit(ManagerEvent::DownloadRemoved { id });
        }
        Ok(())
    }

    // ------------------------------------------------------------------ settings

    pub fn update_settings_sync(&self, mut new: Settings) -> Result<Vec<String>> {
        let problems = new.validate();
        self.inner.db.save_settings(&new)?;
        let client = http::build_client(&new)?;
        *self.inner.client.write() = client;
        self.inner.global_limiter.set_rate(new.global_speed_limit_bps);
        let torrent = self.inner.torrent.clone();
        let (down, up) = (new.global_speed_limit_bps, new.global_upload_limit_bps);
        tokio::spawn(async move { torrent.set_global_limits(down, up).await });
        *self.inner.settings.write() = Arc::new(new);
        self.inner.wake.notify_one();
        Ok(problems)
    }

    pub async fn update_settings(&self, new: Settings) -> Result<Vec<String>> {
        self.update_settings_sync(new)
    }

    pub fn categories(&self) -> Result<Vec<Category>> {
        self.inner.db.categories()
    }

    pub fn upsert_category(&self, c: &Category) -> Result<()> {
        self.inner.db.upsert_category(c)
    }

    pub fn delete_category(&self, name: &str) -> Result<()> {
        self.inner.db.delete_category(name)
    }

    // ------------------------------------------------------------------ inspection / engines

    pub async fn inspect_url(&self, url: &str, engine: Option<EngineKind>) -> Result<UrlInspection> {
        self.inner.inspect(url, engine).await
    }

    pub async fn inspect_torrent_file(&self, bytes: Vec<u8>) -> Result<TorrentInfo> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(DownloadError::new(ErrorKind::Unknown, "Torrent file is too large"));
        }
        let s = self.settings();
        self.inner.torrent.inspect(TorrentSource::Bytes(bytes), &s, Duration::from_secs(10)).await
    }

    pub async fn engine_status(&self) -> (ToolStatus, ToolStatus, TorrentEngineStatus) {
        let s = self.settings();
        (
            tools::ytdlp_status(&self.inner.tools, &s.ytdlp_path).await,
            tools::ffmpeg_status(&self.inner.tools, &s.ffmpeg_path).await,
            self.inner.torrent.status().await,
        )
    }

    /// Installs yt-dlp (official release, checksum-verified) into `dest_dir`.
    pub async fn install_ytdlp(&self, dest_dir: &Path) -> Result<PathBuf> {
        let client = self.inner.client.read().clone();
        tools::install_ytdlp(&client, dest_dir).await
    }

    pub async fn update_ytdlp(&self) -> Result<String> {
        let s = self.settings();
        self.inner.video.self_update(&s.ytdlp_path).await
    }

    /// Re-checks completed downloads whose files were moved or deleted.
    pub fn verify_completed(&self) -> Result<()> {
        for d in self.inner.db.list_downloads()? {
            if d.status == DownloadStatus::Completed {
                let missing = !d.full_path().exists();
                if missing != d.file_missing {
                    self.inner.modify(&d.id, |x| x.file_missing = missing)?;
                }
            }
        }
        Ok(())
    }

    /// Stops all transfers, persisting state so they resume on next launch.
    pub async fn shutdown(&self) {
        let inner = &self.inner;
        inner.shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
        let ids: Vec<String> = inner.jobs.lock().keys().cloned().collect();
        for id in &ids {
            let _ = inner.pause(id, Intent::Requeue);
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while !inner.jobs.lock().is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        inner.sample_progress(true);
        inner.torrent.shutdown().await;
    }
}

impl Inner {
    fn engine(&self, kind: EngineKind) -> Arc<dyn Engine> {
        match kind {
            EngineKind::Http => self.http.clone(),
            EngineKind::Torrent => Arc::new(self.torrent.clone()),
            EngineKind::Video => self.video.clone(),
        }
    }

    fn settings(&self) -> Arc<Settings> {
        self.settings.read().clone()
    }

    fn require(&self, id: &str) -> Result<Download> {
        if let Some(j) = self.jobs.lock().get(id) {
            return Ok(j.download.clone());
        }
        self.db.get_download(id)?.ok_or_else(|| DownloadError::new(ErrorKind::Unknown, "Download not found"))
    }

    fn emit_updated(&self, id: &str) {
        if let Ok(Some(d)) = self.current(id) {
            self.sink.emit(ManagerEvent::DownloadUpdated { download: d });
        }
    }

    fn current(&self, id: &str) -> Result<Option<Download>> {
        if let Some(j) = self.jobs.lock().get(id) {
            return Ok(Some(j.download.clone()));
        }
        self.db.get_download(id)
    }

    /// Applies a non-status change and persists it.
    fn modify(&self, id: &str, f: impl FnOnce(&mut Download)) -> Result<()> {
        let mut f = Some(f);
        let updated = {
            let mut jobs = self.jobs.lock();
            if let Some(j) = jobs.get_mut(id) {
                (f.take().unwrap())(&mut j.download);
                j.download.updated_at = now();
                Some(j.download.clone())
            } else {
                None
            }
        };
        let d = match updated {
            Some(d) => d,
            None => {
                let mut d = self.db.get_download(id)?.ok_or_else(|| DownloadError::new(ErrorKind::Unknown, "Download not found"))?;
                (f.take().unwrap())(&mut d);
                d.updated_at = now();
                d
            }
        };
        self.db.update_download(&d)?;
        self.sink.emit(ManagerEvent::DownloadUpdated { download: d });
        Ok(())
    }

    /// The only place a status changes. Invalid transitions are rejected.
    fn transition(&self, id: &str, to: DownloadStatus, f: impl FnOnce(&mut Download)) -> Result<Download> {
        let mut d = self.require(id)?;
        if !d.status.can_transition_to(to) {
            return Err(DownloadError::new(ErrorKind::Unknown, format!("Invalid state change: {} → {}", d.status, to)));
        }
        tracing::debug!(id, from = %d.status, to = %to, "transition");
        d.status = to;
        if !to.is_running() {
            d.speed_bps = 0;
            d.upload_bps = 0;
            d.eta_seconds = None;
        }
        f(&mut d);
        d.updated_at = now();
        if let Some(j) = self.jobs.lock().get_mut(id) {
            j.download = d.clone();
        }
        self.db.update_download(&d)?;
        let event = match to {
            DownloadStatus::Completed => ManagerEvent::DownloadCompleted { download: d.clone() },
            DownloadStatus::Failed => ManagerEvent::DownloadFailed { download: d.clone() },
            _ => ManagerEvent::DownloadUpdated { download: d.clone() },
        };
        self.sink.emit(event);
        Ok(d)
    }

    fn pause(&self, id: &str, intent: Intent) -> Result<()> {
        {
            let mut jobs = self.jobs.lock();
            if let Some(j) = jobs.get_mut(id) {
                // A stronger intent (cancel/remove) overrides a pause.
                let keep = matches!((j.intent, intent), (Some(Intent::Cancel), Intent::Pause) | (Some(Intent::Remove { .. }), _));
                if !keep {
                    j.intent = Some(intent);
                }
                let ctl = if matches!(j.intent, Some(Intent::Cancel) | Some(Intent::Remove { .. })) { Control::Cancel } else { Control::Pause };
                let _ = j.control.send(ctl);
                return Ok(());
            }
        }
        let d = self.require(id)?;
        if let (DownloadStatus::Queued, Intent::Pause) = (d.status, intent) {
            self.retry_at.lock().remove(id);
            self.transition(id, DownloadStatus::Paused, |_| {})?;
            self.db.add_event(id, "paused", None)?;
        }
        Ok(())
    }

    async fn finish_remove(&self, d: &Download, delete_files: bool) -> Result<()> {
        if d.status != DownloadStatus::Completed || delete_files {
            self.engine(d.engine).cleanup(d, delete_files).await;
        }
        self.retry_at.lock().remove(&d.id);
        self.db.delete_download(&d.id)?;
        self.sink.emit(ManagerEvent::DownloadRemoved { id: d.id.clone() });
        Ok(())
    }

    // ------------------------------------------------------------------ add

    async fn add(self: &Arc<Self>, req: AddDownloadRequest, source: AddSource) -> Result<Download> {
        let settings = self.settings();
        let torrent_bytes = match &req.torrent_base64 {
            Some(b64) => {
                if b64.len() > 22 * 1024 * 1024 {
                    return Err(DownloadError::new(ErrorKind::Unknown, "Torrent file is too large"));
                }
                Some(base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| DownloadError::invalid_url("Invalid torrent file data"))?)
            }
            None => None,
        };

        let (url, detected) = if torrent_bytes.is_some() {
            (None, EngineKind::Torrent)
        } else {
            let (u, k) = detector::detect(&req.url)?;
            (Some(u), k)
        };
        let engine = req.engine.unwrap_or(detected);
        if let Some(u) = &url {
            if engine == EngineKind::Torrent && u.scheme() == "ftp" {
                return Err(DownloadError::invalid_url("FTP torrent URLs are not supported"));
            }
            if engine != EngineKind::Torrent && u.scheme() == "magnet" {
                return Err(DownloadError::invalid_url("Magnet links can only be downloaded with the torrent engine"));
            }
            if u.scheme() == "ftp" {
                return Err(DownloadError::invalid_url("FTP downloads are not supported in this version"));
            }
        }

        // The same URL clicked twice in the browser should not start twice.
        if let Some(u) = &url {
            if let Some(existing) = self.db.find_active_by_url(u.as_str())? {
                return Ok(existing);
            }
        }

        // Destination: the browser can never choose a folder.
        let save_dir = match (&req.save_dir, source) {
            (Some(dir), AddSource::User) if !dir.trim().is_empty() => security::ensure_approved_dir(Path::new(dir.trim()), &settings.approved_dirs())?,
            _ => PathBuf::from(&settings.default_download_dir),
        };

        let explicit = req.filename.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(security::sanitize_filename);
        let provisional = explicit.clone().unwrap_or_else(|| match (&url, engine) {
            (Some(u), EngineKind::Torrent) if u.scheme() == "magnet" => torrent::magnet_display_name(u.as_str()).unwrap_or_else(|| "torrent".into()),
            (Some(u), EngineKind::Video) => security::sanitize_filename(u.host_str().unwrap_or("video")),
            (Some(u), _) => security::filename_from_url(u).unwrap_or_else(|| "download".into()),
            (None, _) => "torrent".into(),
        });
        let category = match req.category.as_deref() {
            Some(c) if self.db.categories()?.iter().any(|x| x.name == c) => c.to_string(),
            _ => self.db.detect_category(&provisional, engine),
        };
        let save_dir = if settings.category_subfolders && req.save_dir.is_none() {
            let sub = self.db.categories()?.into_iter().find(|c| c.name == category).map(|c| c.subfolder).unwrap_or_default();
            if sub.is_empty() { save_dir } else { save_dir.join(security::sanitize_filename(&sub)) }
        } else {
            save_dir
        };
        let save_dir_s = save_dir.to_string_lossy().to_string();
        let filename = if engine == EngineKind::Http {
            let db = self.db.clone();
            let dir = save_dir_s.clone();
            let reserved = move |n: &str| db.filename_reserved(&dir, n, None).unwrap_or(false);
            security::resolve_duplicate(&save_dir, &provisional, settings.duplicate_policy, &reserved)
        } else {
            provisional
        };

        if let Some(at) = &req.scheduled_at {
            chrono::DateTime::parse_from_rfc3339(at).map_err(|e| DownloadError::new(ErrorKind::Unknown, "Invalid schedule time").with_detail(e.to_string()))?;
        }
        let referrer = req.referrer.as_deref().and_then(|r| url::Url::parse(r).ok()).filter(|r| r.scheme() == "http" || r.scheme() == "https").map(|r| r.to_string());
        let mut options = req.engine_options.clone();
        options.explicit_filename = explicit.is_some();
        if let Some(c) = options.connections {
            options.connections = Some(c.clamp(1, 16));
        }
        let t = now();
        let url_s = url.as_ref().map(|u| u.to_string()).unwrap_or_else(|| format!("torrent-file:{filename}"));
        let d = Download {
            id: uuid::Uuid::new_v4().to_string(),
            url: url_s.clone(),
            original_url: url_s,
            referrer,
            filename,
            save_dir: save_dir_s,
            category,
            engine,
            status: if req.start_paused { DownloadStatus::Paused } else { DownloadStatus::Queued },
            priority: req.priority.unwrap_or(Priority::Normal),
            queue_position: self.db.next_queue_position()?,
            total_bytes: None,
            downloaded_bytes: 0,
            speed_bps: 0,
            upload_bps: 0,
            eta_seconds: None,
            resumable: None,
            speed_limit_bps: None,
            connections: 1,
            error_kind: None,
            error_message: None,
            error_detail: None,
            retry_count: 0,
            engine_options: options,
            peers: None,
            seeds: None,
            info_hash: None,
            title: None,
            thumbnail: None,
            created_at: t.clone(),
            started_at: None,
            completed_at: None,
            updated_at: t,
            scheduled_at: req.scheduled_at.clone(),
            file_missing: false,
        };
        self.db.insert_download(&d)?;
        if let Some(b) = &torrent_bytes {
            self.db.set_torrent_data(&d.id, b)?;
        }
        self.db.add_event(&d.id, "created", Some(&format!("{} via {}", redact_url(&d.url), if source == AddSource::Browser { "browser" } else { "app" })))?;
        tracing::info!(id = %d.id, engine = d.engine.as_str(), url = %redact_url(&d.url), "download added");
        self.sink.emit(ManagerEvent::DownloadCreated { download: d.clone() });
        self.wake.notify_one();
        Ok(d)
    }

    // ------------------------------------------------------------------ inspection

    async fn inspect(&self, input: &str, override_engine: Option<EngineKind>) -> Result<UrlInspection> {
        let settings = self.settings();
        let (url, detected) = detector::detect(input)?;
        let engine = override_engine.unwrap_or(detected);
        let mut out = UrlInspection {
            url: url.to_string(),
            final_url: None,
            engine,
            alternatives: vec![],
            filename: None,
            total_bytes: None,
            content_type: None,
            resumable: None,
            category: "General".into(),
            video: None,
            torrent: None,
            warning: None,
        };
        match engine {
            EngineKind::Torrent => {
                out.alternatives = if url.scheme() == "magnet" { vec![] } else { vec![EngineKind::Http] };
                let src = if url.scheme() == "magnet" { TorrentSource::Magnet(url.to_string()) } else { TorrentSource::Url(url.to_string()) };
                if url.scheme() == "magnet" {
                    out.filename = torrent::magnet_display_name(url.as_str());
                }
                match self.torrent.inspect(src, &settings, Duration::from_secs(25)).await {
                    Ok(info) => {
                        out.filename = Some(security::sanitize_filename(&info.name));
                        out.total_bytes = Some(info.total_bytes);
                        out.torrent = Some(info);
                        out.resumable = Some(true);
                    }
                    Err(e) => out.warning = Some(e.into()),
                }
            }
            EngineKind::Video => {
                out.alternatives = vec![EngineKind::Http];
                match self.video.inspect(url.as_str(), &settings.ytdlp_path, &settings.ffmpeg_path).await {
                    Ok(info) => {
                        out.filename = Some(security::sanitize_filename(&info.title));
                        out.total_bytes = info.presets.first().and_then(|p| p.approx_size);
                        out.video = Some(info);
                    }
                    Err(e) => out.warning = Some(e.into()),
                }
            }
            EngineKind::Http => {
                if url.scheme() == "ftp" {
                    out.warning = Some(InspectionError { kind: ErrorKind::InvalidUrl, message: "FTP downloads are not supported in this version".into(), detail: None });
                } else {
                    let client = self.client.read().clone();
                    match http::probe(&client, url.as_str(), None).await {
                        Ok(p) => {
                            out.final_url = Some(p.final_url.clone());
                            out.filename = p.filename.clone();
                            out.total_bytes = p.total_bytes;
                            out.resumable = Some(p.resumable);
                            out.content_type = p.content_type.clone();
                            let ct = p.content_type.as_deref().unwrap_or("");
                            if ct == "application/x-bittorrent" && override_engine.is_none() {
                                out.engine = EngineKind::Torrent;
                                out.alternatives = vec![EngineKind::Http];
                                if let Ok(info) = self.torrent.inspect(TorrentSource::Url(url.to_string()), &settings, Duration::from_secs(20)).await {
                                    out.total_bytes = Some(info.total_bytes);
                                    out.filename = Some(security::sanitize_filename(&info.name));
                                    out.torrent = Some(info);
                                }
                            } else if ct == "text/html" && override_engine.is_none() {
                                // A web page: maybe a media page the extractor understands.
                                out.alternatives = vec![EngineKind::Video];
                                if self.tools.find("yt-dlp", &settings.ytdlp_path).is_some() {
                                    if let Ok(Ok(info)) = tokio::time::timeout(Duration::from_secs(30), self.video.inspect(url.as_str(), &settings.ytdlp_path, &settings.ffmpeg_path)).await {
                                        if !info.formats.is_empty() {
                                            out.engine = EngineKind::Video;
                                            out.alternatives = vec![EngineKind::Http];
                                            out.filename = Some(security::sanitize_filename(&info.title));
                                            out.total_bytes = info.presets.first().and_then(|p| p.approx_size);
                                            out.video = Some(info);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => out.warning = Some(e.into()),
                    }
                }
            }
        }
        out.category = self.db.detect_category(out.filename.as_deref().unwrap_or(""), out.engine);
        Ok(out)
    }

    // ------------------------------------------------------------------ recovery

    fn recover(&self) -> Result<()> {
        let settings = self.settings();
        for d in self.db.list_downloads()? {
            match d.status {
                DownloadStatus::Preparing | DownloadStatus::Downloading => {
                    // Interrupted by a crash / forced exit.
                    let mut r = d.clone();
                    r.status = if settings.auto_resume_on_startup { DownloadStatus::Queued } else { DownloadStatus::Paused };
                    r.speed_bps = 0;
                    r.upload_bps = 0;
                    r.eta_seconds = None;
                    r.downloaded_bytes = self.verified_bytes(&d);
                    r.updated_at = now();
                    self.db.update_download(&r)?;
                    self.db.add_event(&d.id, "recovered", Some("Interrupted download restored after restart"))?;
                    tracing::info!(id = %d.id, "recovered interrupted download");
                }
                DownloadStatus::Queued | DownloadStatus::Paused if d.engine == EngineKind::Http => {
                    let bytes = self.verified_bytes(&d);
                    if bytes != d.downloaded_bytes {
                        self.db.update_progress(&d.id, bytes, None)?;
                    }
                }
                DownloadStatus::Completed => {
                    let missing = !d.full_path().exists();
                    if missing != d.file_missing {
                        let mut r = d.clone();
                        r.file_missing = missing;
                        self.db.update_download(&r)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bytes actually present on disk for a partial HTTP download.
    fn verified_bytes(&self, d: &Download) -> u64 {
        if d.engine != EngineKind::Http {
            return d.downloaded_bytes;
        }
        let segs = self.db.load_segments(&d.id).unwrap_or_default();
        if !segs.is_empty() {
            return segs.iter().map(|s| s.downloaded.min(s.len())).sum();
        }
        std::fs::metadata(http::part_path(Path::new(&d.save_dir), &d.filename)).map(|m| m.len()).unwrap_or(0)
    }

    // ------------------------------------------------------------------ queue

    fn schedule_pass(self: &Arc<Self>) {
        if self.shutting_down.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let settings = self.settings();
        let allowed = crate::scheduler::window_allows(&settings.schedule, chrono::Local::now().naive_local());
        if !allowed {
            // Outside the window: send running downloads back to the queue.
            let running: Vec<String> = self.jobs.lock().iter().filter(|(_, j)| j.intent.is_none()).map(|(k, _)| k.clone()).collect();
            for id in running {
                let _ = self.pause(&id, Intent::Requeue);
            }
            return;
        }
        let active = self.jobs.lock().len();
        let max = settings.max_concurrent_downloads as usize;
        let queued = match self.db.queued_in_order() {
            Ok(q) => q,
            Err(e) => {
                tracing::error!(error = %e, "cannot read queue");
                return;
            }
        };
        let now_utc = chrono::Utc::now();
        let mut free = max.saturating_sub(active);
        let mut waiting = 0;
        for d in queued {
            if self.jobs.lock().contains_key(&d.id) {
                continue;
            }
            if let Some(at) = &d.scheduled_at {
                if chrono::DateTime::parse_from_rfc3339(at).map(|t| t > now_utc).unwrap_or(false) {
                    waiting += 1;
                    continue;
                }
            }
            if self.retry_at.lock().get(&d.id).is_some_and(|t| *t > Instant::now()) {
                waiting += 1;
                continue;
            }
            if free == 0 {
                waiting += 1;
                continue;
            }
            if let Err(e) = self.start_job(d) {
                tracing::error!(error = %e, "failed to start job");
                continue;
            }
            free -= 1;
        }
        // Finished = nothing running, nothing waiting (queued, scheduled or retrying).
        let idle = self.jobs.lock().is_empty() && waiting == 0;
        if idle && self.had_activity.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.sink.emit(ManagerEvent::QueueFinished);
        }
    }

    fn start_job(self: &Arc<Self>, d: Download) -> Result<()> {
        let id = d.id.clone();
        let started = d.started_at.clone().unwrap_or_else(now);
        let d = self.transition(&id, DownloadStatus::Preparing, |x| {
            x.started_at = Some(started);
            x.error_kind = None;
            x.error_message = None;
            x.error_detail = None;
        })?;
        let (ctl_tx, ctl_rx) = watch::channel(Control::Run);
        let (meta_tx, meta_rx) = mpsc::unbounded_channel();
        let progress = Arc::new(ProgressCell::default());
        progress.set(crate::engines::EngineProgress { downloaded: d.downloaded_bytes, total: d.total_bytes, ..Default::default() });
        let limiter = Arc::new(RateLimiter::new(d.speed_limit_bps));
        let ctx = JobContext {
            download: d.clone(),
            settings: self.settings(),
            db: self.db.clone(),
            http: self.client.read().clone(),
            control: ctl_rx,
            progress: progress.clone(),
            meta: meta_tx,
            global_limiter: self.global_limiter.clone(),
            download_limiter: limiter.clone(),
        };
        self.jobs.lock().insert(
            id.clone(),
            Job { control: ctl_tx, progress, limiter, download: d.clone(), intent: None, last_progress: Default::default() },
        );
        let engine = self.engine(d.engine);
        let me = self.clone();
        tokio::spawn(async move {
            let fut = engine.run(ctx);
            let result = me.drive(&id, fut, meta_rx).await;
            me.finish_job(&id, result).await;
        });
        Ok(())
    }

    async fn drive(
        &self,
        id: &str,
        fut: futures::future::BoxFuture<'static, Result<EngineOutcome>>,
        mut meta_rx: mpsc::UnboundedReceiver<MetaUpdate>,
    ) -> Result<EngineOutcome> {
        // Run the engine on its own task so a panic is contained.
        let mut handle = tokio::spawn(fut);
        loop {
            tokio::select! {
                res = &mut handle => {
                    while let Ok(m) = meta_rx.try_recv() {
                        self.apply_meta(id, m);
                    }
                    return match res {
                        Ok(r) => r,
                        Err(e) => Err(DownloadError::new(ErrorKind::Unknown, "Download engine crashed").with_detail(e.to_string())),
                    };
                }
                Some(m) = meta_rx.recv() => self.apply_meta(id, m),
            }
        }
    }

    fn apply_meta(&self, id: &str, m: MetaUpdate) {
        if let Some(ev) = &m.event {
            let _ = self.db.add_event(id, "info", Some(ev));
        }
        let mut became_downloading = false;
        let updated = {
            let mut jobs = self.jobs.lock();
            let Some(j) = jobs.get_mut(id) else { return };
            let d = &mut j.download;
            if let Some(f) = m.filename {
                d.filename = f;
            }
            if let Some(u) = m.url {
                d.url = u;
            }
            if m.total_bytes.is_some() {
                d.total_bytes = m.total_bytes;
            }
            if m.resumable.is_some() {
                d.resumable = m.resumable;
            }
            if let Some(t) = m.title {
                d.title = Some(t);
            }
            if let Some(t) = m.thumbnail {
                d.thumbnail = Some(t);
            }
            if let Some(h) = m.info_hash {
                d.info_hash = Some(h);
            }
            if let Some(c) = m.connections {
                d.connections = c;
            }
            if m.downloading && d.status == DownloadStatus::Preparing {
                became_downloading = true;
            }
            d.updated_at = now();
            d.clone()
        };
        let _ = self.db.update_download(&updated);
        if became_downloading {
            let _ = self.transition(id, DownloadStatus::Downloading, |_| {});
        } else {
            self.sink.emit(ManagerEvent::DownloadUpdated { download: updated });
        }
    }

    async fn finish_job(self: &Arc<Self>, id: &str, result: Result<EngineOutcome>) {
        let Some(job) = self.jobs.lock().remove(id) else { return };
        let p = job.progress.get();
        let intent = job.intent;
        let mut base = job.download.clone();
        base.downloaded_bytes = p.downloaded;
        if p.total.is_some() {
            base.total_bytes = p.total;
        }
        let _ = self.db.update_download(&base);
        let settings = self.settings();
        let engine = self.engine(base.engine);

        let outcome: Result<()> = async {
            match (result, intent) {
                (Ok(EngineOutcome::Completed { filename }), _) => {
                    let size = std::fs::metadata(Path::new(&base.save_dir).join(&filename)).ok().filter(|m| m.is_file()).map(|m| m.len());
                    self.transition(id, DownloadStatus::Completed, |d| {
                        d.filename = filename.clone();
                        if let Some(s) = size {
                            d.total_bytes = Some(s);
                            d.downloaded_bytes = s;
                        } else if let Some(t) = d.total_bytes {
                            d.downloaded_bytes = t;
                        }
                        d.completed_at = Some(now());
                        d.peers = None;
                        d.seeds = None;
                        d.scheduled_at = None;
                    })?;
                    self.db.add_event(id, "completed", None)?;
                    // Only a completion arms "queue finished" (never a pause/cancel/failure),
                    // because it can trigger the user's after-queue sleep/shutdown action.
                    self.had_activity.store(true, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(id, "download completed");
                }
                (_, Some(Intent::Remove { delete_files })) => {
                    let d = self.require(id)?;
                    self.finish_remove(&d, delete_files).await?;
                }
                (_, Some(Intent::Cancel)) => {
                    let d = self.require(id)?;
                    engine.cleanup(&d, false).await;
                    self.db.clear_segments(id)?;
                    self.transition(id, DownloadStatus::Cancelled, |d| d.downloaded_bytes = 0)?;
                    self.db.add_event(id, "cancelled", None)?;
                }
                (Ok(EngineOutcome::Stopped), Some(Intent::Requeue)) | (Err(_), Some(Intent::Requeue)) => {
                    let to = if self.shutting_down.load(std::sync::atomic::Ordering::Relaxed) && !settings.auto_resume_on_startup {
                        DownloadStatus::Paused
                    } else {
                        DownloadStatus::Queued
                    };
                    self.transition(id, to, |_| {})?;
                }
                (Ok(EngineOutcome::Stopped), _) | (Err(_), Some(Intent::Pause)) => {
                    self.transition(id, DownloadStatus::Paused, |_| {})?;
                    self.db.add_event(id, "paused", None)?;
                }
                (Err(e), None) => {
                    let current = self.require(id)?;
                    let policy = RetryPolicy::new(settings.retry_count, settings.retry_delay_secs);
                    let attempt = current.retry_count + 1;
                    tracing::warn!(id, kind = e.kind.as_str(), error = %e, detail = ?e.detail, "download error");
                    match (e.is_retryable(), policy.delay_for(attempt)) {
                        (true, Some(delay)) if !self.shutting_down.load(std::sync::atomic::Ordering::Relaxed) => {
                            self.retry_at.lock().insert(id.to_string(), Instant::now() + delay);
                            self.transition(id, DownloadStatus::Queued, |d| {
                                d.retry_count = attempt;
                                d.error_kind = Some(e.kind);
                                d.error_message = Some(e.message.clone());
                                d.error_detail = e.detail.clone();
                            })?;
                            self.db.add_event(id, "retry_scheduled", Some(&format!("{} — retry {attempt}/{} in {}s", e.message, settings.retry_count, delay.as_secs())))?;
                            let me = self.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(delay).await;
                                me.wake.notify_one();
                            });
                        }
                        _ => {
                            self.transition(id, DownloadStatus::Failed, |d| {
                                d.error_kind = Some(e.kind);
                                d.error_message = Some(e.message.clone());
                                d.error_detail = e.detail.clone();
                            })?;
                            self.db.add_event(id, "failed", Some(&format!("{}{}", e.message, e.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default())))?;
                        }
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = outcome {
            tracing::error!(id, error = %e, detail = ?e.detail, "failed to finalise job");
        }
        self.wake.notify_one();
    }

    // ------------------------------------------------------------------ progress

    fn sample_progress(&self, persist: bool) {
        let mut updates = Vec::new();
        let mut to_persist = Vec::new();
        {
            let mut jobs = self.jobs.lock();
            for (id, j) in jobs.iter_mut() {
                let p = j.progress.get();
                if p != j.last_progress {
                    j.last_progress = p.clone();
                    let d = &mut j.download;
                    d.downloaded_bytes = p.downloaded;
                    if p.total.is_some() {
                        d.total_bytes = p.total;
                    }
                    d.speed_bps = p.speed_bps;
                    d.upload_bps = p.upload_bps;
                    d.eta_seconds = p.eta_seconds;
                    d.peers = p.peers;
                    d.seeds = p.seeds;
                    updates.push(ProgressUpdate {
                        download_id: id.clone(),
                        status: d.status,
                        downloaded_bytes: d.downloaded_bytes,
                        total_bytes: d.total_bytes,
                        speed_bytes_per_second: d.speed_bps,
                        upload_bytes_per_second: d.upload_bps,
                        eta_seconds: d.eta_seconds,
                        peers: d.peers,
                        seeds: d.seeds,
                    });
                }
                if persist {
                    to_persist.push((id.clone(), j.download.downloaded_bytes, j.download.total_bytes));
                }
            }
        }
        for (id, done, total) in to_persist {
            if let Err(e) = self.db.update_progress(&id, done, total) {
                tracing::warn!(error = %e, "progress persist failed");
            }
        }
        if !updates.is_empty() {
            self.sink.emit(ManagerEvent::DownloadProgress { updates });
        }
    }
}

/// IDs of downloads in a given set of statuses (helper for the shell).
pub fn ids_with_status(list: &[Download], statuses: &[DownloadStatus]) -> HashSet<String> {
    list.iter().filter(|d| statuses.contains(&d.status)).map(|d| d.id.clone()).collect()
}

/// Inspect result for a local .torrent file, used by the Add dialog.
pub type TorrentPreview = TorrentInfo;
pub type VideoPreview = VideoInfo;
