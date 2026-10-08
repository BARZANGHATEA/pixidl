//! HTTP/HTTPS engine.
//!
//! - streams to `<name>.part`, never buffering whole files in memory
//! - resumes with `Range` + `If-Range` (ETag / Last-Modified) so a changed
//!   resource is detected instead of silently corrupting the file
//! - detects servers without range support and restarts honestly
//! - splits large resumable files into segments downloaded over parallel
//!   connections; segment progress is persisted so a crash can resume
//! - enforces the per-download and global rate limits
//! - renames `.part` → final name only after the byte count is verified

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderValue};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;

use super::{available_space, eta, Control, Engine, EngineOutcome, EngineProgress, JobContext, MetaUpdate, SpeedMeter};
use crate::db::{Segment, Validators};
use crate::error::{DownloadError, Result};
use crate::ratelimit::RateLimiter;
use crate::security::{self, DuplicatePolicy};
use crate::settings::{ProxyMode, Settings};
use crate::types::{Download, EngineKind, ErrorKind};

/// Files smaller than this are always downloaded over one connection.
const MULTI_CONNECTION_THRESHOLD: u64 = 2 * 1024 * 1024;
const MIN_SEGMENT: u64 = 512 * 1024;
const WRITE_BUFFER: usize = 512 * 1024;
const SEGMENT_ATTEMPTS: u32 = 4;

pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) pixidl/",
    env!("CARGO_PKG_VERSION")
);

/// Builds the shared HTTP client from settings (proxy, timeouts).
/// Transparent decompression is disabled on purpose: byte ranges and sizes
/// must refer to the bytes on the wire.
pub fn build_client(s: &Settings) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(s.connect_timeout_secs as u64))
        .redirect(reqwest::redirect::Policy::limited(10))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_max_idle_per_host(16)
        .https_only(false);
    match s.proxy_mode {
        ProxyMode::None => b = b.no_proxy(),
        ProxyMode::System => {} // reqwest honours HTTP(S)_PROXY / ALL_PROXY
        ProxyMode::Manual => {
            let p = reqwest::Proxy::all(s.proxy_url.trim())
                .map_err(|e| DownloadError::new(ErrorKind::NetworkUnavailable, "Invalid proxy").with_detail(e.to_string()))?;
            b = b.proxy(p);
        }
    }
    b.build().map_err(|e| DownloadError::new(ErrorKind::Unknown, "Cannot create HTTP client").with_detail(e.to_string()))
}

pub struct HttpEngine;

impl Engine for HttpEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Http
    }

    fn run(&self, ctx: JobContext) -> BoxFuture<'static, Result<EngineOutcome>> {
        Box::pin(run(ctx))
    }

    fn cleanup(&self, d: &Download, delete_completed: bool) -> BoxFuture<'static, ()> {
        let part = part_path(Path::new(&d.save_dir), &d.filename);
        let full = d.full_path();
        let completed = d.status == crate::types::DownloadStatus::Completed;
        Box::pin(async move {
            let _ = tokio::fs::remove_file(&part).await;
            if delete_completed && completed {
                let _ = tokio::fs::remove_file(&full).await;
            }
        })
    }
}

pub fn part_path(dir: &Path, filename: &str) -> PathBuf {
    dir.join(format!("{filename}.part"))
}

/// Result of probing a URL without downloading it.
#[derive(Debug, Clone, Default)]
pub struct HttpProbe {
    pub final_url: String,
    pub filename: Option<String>,
    pub total_bytes: Option<u64>,
    pub resumable: bool,
    pub content_type: Option<String>,
}

/// Requests the first byte to learn size, name, type and range support.
pub async fn probe(client: &reqwest::Client, url: &str, referrer: Option<&str>) -> Result<HttpProbe> {
    let mut req = client.get(url).header(header::RANGE, "bytes=0-0").header(header::ACCEPT_ENCODING, "identity");
    if let Some(r) = referrer.filter(|r| r.starts_with("http")) {
        req = req.header(header::REFERER, r);
    }
    let resp = req.send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(DownloadError::from_status(status));
    }
    let headers = resp.headers().clone();
    let final_url = resp.url().clone();
    let content_type = header_str(&headers, header::CONTENT_TYPE).map(|s| s.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
    let (total, resumable) = if status == 206 {
        (parse_content_range_total(&headers), true)
    } else {
        (resp.content_length(), accepts_ranges(&headers))
    };
    drop(resp);
    Ok(HttpProbe {
        final_url: final_url.to_string(),
        filename: detect_filename(&headers, &final_url, content_type.as_deref()),
        total_bytes: total,
        resumable,
        content_type,
    })
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}

fn accepts_ranges(h: &HeaderMap) -> bool {
    header_str(h, header::ACCEPT_RANGES).is_some_and(|v| v.to_ascii_lowercase().contains("bytes"))
}

/// `Content-Range: bytes 0-0/12345` → 12345
pub fn parse_content_range_total(h: &HeaderMap) -> Option<u64> {
    let v = header_str(h, header::CONTENT_RANGE)?;
    v.rsplit('/').next()?.trim().parse().ok()
}

/// `Content-Range: bytes 100-199/1000` → (100, 199)
fn parse_content_range_span(h: &HeaderMap) -> Option<(u64, u64)> {
    let v = header_str(h, header::CONTENT_RANGE)?;
    let span = v.trim().strip_prefix("bytes")?.trim().split('/').next()?;
    let (a, b) = span.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-bittorrent" => "torrent",
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => "exe",
        "application/x-msi" => "msi",
        "application/x-iso9660-image" => "iso",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        _ => return None,
    })
}

/// Content-Disposition → URL path → fallback, plus an extension from the MIME
/// type when the name has none.
pub fn detect_filename(h: &HeaderMap, final_url: &url::Url, content_type: Option<&str>) -> Option<String> {
    let name = header_str(h, header::CONTENT_DISPOSITION)
        .and_then(|cd| security::filename_from_content_disposition(&cd))
        .or_else(|| security::filename_from_url(final_url))?;
    if security::extension_of(&name).is_empty() {
        if let Some(ext) = content_type.and_then(ext_for_mime) {
            return Some(format!("{name}.{ext}"));
        }
    }
    Some(name)
}

fn usable_etag(etag: &str) -> bool {
    // Weak validators must not be used with If-Range (RFC 9110 §13.1.5).
    !etag.starts_with("W/")
}

/// Waits for rate-limit tokens, aborting if the job is stopped meanwhile.
async fn throttle(n: u64, per: &RateLimiter, global: &RateLimiter, control: &mut watch::Receiver<Control>) -> Option<Control> {
    let acquire = async {
        per.acquire(n).await;
        global.acquire(n).await;
    };
    tokio::select! {
        biased;
        c = JobContext::stopped(control) => Some(c),
        _ = acquire => None,
    }
}

/// Buffered writer that tracks how many bytes have been handed to the OS.
struct ChunkWriter {
    file: tokio::fs::File,
    buf: Vec<u8>,
    committed: u64,
}

impl ChunkWriter {
    fn new(file: tokio::fs::File) -> Self {
        Self { file, buf: Vec::with_capacity(WRITE_BUFFER), committed: 0 }
    }
    async fn push(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= WRITE_BUFFER {
            self.flush().await?;
        }
        Ok(())
    }
    async fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            self.file.write_all(&self.buf).await?;
            self.committed += self.buf.len() as u64;
            self.buf.clear();
        }
        self.file.flush().await
    }
    fn pending(&self) -> u64 {
        self.buf.len() as u64
    }
}

fn io_err(e: std::io::Error) -> DownloadError {
    DownloadError::from_io(&e)
}

fn request(ctx_http: &reqwest::Client, url: &str, referrer: Option<&str>) -> reqwest::RequestBuilder {
    let mut r = ctx_http.get(url).header(header::ACCEPT_ENCODING, "identity");
    if let Some(rf) = referrer.filter(|r| r.starts_with("http")) {
        r = r.header(header::REFERER, rf);
    }
    r
}

async fn run(mut ctx: JobContext) -> Result<EngineOutcome> {
    let d = ctx.download.clone();
    let url = security::validate_url(&d.url)?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(DownloadError::new(ErrorKind::InvalidUrl, "FTP is not supported by the HTTP engine")
            .with_detail("Only http:// and https:// URLs can be downloaded by this engine."));
    }
    let save_dir = PathBuf::from(&d.save_dir);
    tokio::fs::create_dir_all(&save_dir).await.map_err(|e| DownloadError::fs("Cannot create destination folder", &e))?;
    let read_timeout = Duration::from_secs(ctx.settings.read_timeout_secs as u64);

    let mut filename = d.filename.clone();
    let segments = ctx.db.load_segments(&d.id)?;
    let part_len = tokio::fs::metadata(part_path(&save_dir, &filename)).await.map(|m| m.len()).unwrap_or(0);
    let validators = ctx.db.validators(&d.id)?;
    let resume_from = if segments.is_empty() { part_len } else { 0 };

    let mut req = request(&ctx.http, &d.url, d.referrer.as_deref()).header(header::RANGE, format!("bytes={resume_from}-"));
    let has_validator = validators.etag.as_deref().is_some_and(usable_etag) || validators.last_modified.is_some();
    if resume_from > 0 || !segments.is_empty() {
        if let Some(etag) = validators.etag.as_deref().filter(|e| usable_etag(e)) {
            req = req.header(header::IF_RANGE, etag);
        } else if let Some(lm) = &validators.last_modified {
            req = req.header(header::IF_RANGE, lm.as_str());
        }
    }

    let resp = tokio::select! {
        biased;
        _ = JobContext::stopped(&mut ctx.control) => return Ok(EngineOutcome::Stopped),
        r = tokio::time::timeout(read_timeout + Duration::from_secs(ctx.settings.connect_timeout_secs as u64), req.send()) => match r {
            Err(_) => return Err(DownloadError::new(ErrorKind::Timeout, "Connection timed out")),
            Ok(r) => r.map_err(|e| DownloadError::from_reqwest(&e))?,
        },
    };
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();

    if status == 416 && resume_from > 0 {
        // Range not satisfiable: either the file is already complete or it changed.
        if parse_content_range_total(&headers) == Some(resume_from) {
            drop(resp);
            return finalize(&ctx, &save_dir, &filename, Some(resume_from)).await;
        }
        drop(resp);
        let _ = tokio::fs::remove_file(part_path(&save_dir, &filename)).await;
        return Err(DownloadError::new(ErrorKind::ResumeNotSupported, "The file on the server changed; partial data was discarded")
            .with_detail("HTTP 416 on resume"));
    }
    if !(200..300).contains(&status) {
        return Err(DownloadError::from_status(status));
    }

    let final_url = resp.url().clone();
    let is_partial = status == 206;
    let content_type = header_str(&headers, header::CONTENT_TYPE).map(|s| s.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
    let total = if is_partial { parse_content_range_total(&headers) } else { resp.content_length() };
    let resumable = is_partial || accepts_ranges(&headers);
    let new_validators = Validators { etag: header_str(&headers, header::ETAG), last_modified: header_str(&headers, header::LAST_MODIFIED) };

    let fresh_start = resume_from == 0 && segments.is_empty();
    let range_ignored = !is_partial && (resume_from > 0 || !segments.is_empty());
    let mut events = Vec::new();
    if range_ignored {
        events.push(if has_validator {
            "The file changed on the server or the server does not support resume; restarting from the beginning".to_string()
        } else {
            "Server does not support resume; restarting from the beginning".to_string()
        });
    }

    // Name detection only before any data has been written for this download.
    if (fresh_start || range_ignored) && !d.engine_options.explicit_filename {
        if let Some(candidate) = detect_filename(&headers, &final_url, content_type.as_deref()) {
            if candidate != filename {
                let id = d.id.clone();
                let dir_s = d.save_dir.clone();
                let db = ctx.db.clone();
                let reserved = move |n: &str| db.filename_reserved(&dir_s, n, Some(&id)).unwrap_or(false);
                let resolved = security::resolve_duplicate(&save_dir, &candidate, ctx.settings.duplicate_policy, &reserved);
                let _ = tokio::fs::remove_file(part_path(&save_dir, &filename)).await;
                filename = resolved;
            }
        }
    }

    // Disk space: what we still need to write.
    if let Some(t) = total {
        let already = if is_partial { resume_from } else { 0 };
        let needed = t.saturating_sub(already);
        if let Some(free) = available_space(&save_dir) {
            if free < needed.saturating_add(8 * 1024 * 1024) {
                return Err(DownloadError::new(ErrorKind::DiskFull, "Not enough disk space").with_detail(format!(
                    "Need {} more bytes, {} available in {}",
                    needed,
                    free,
                    save_dir.display()
                )));
            }
        }
    }

    ctx.db.set_validators(&d.id, &new_validators)?;
    let conns = d.engine_options.connections.unwrap_or(ctx.settings.connections_per_download).clamp(1, 16);
    let use_multi = is_partial
        && conns > 1
        && total.is_some_and(|t| t >= MULTI_CONNECTION_THRESHOLD)
        && (resume_from == 0 || !segments.is_empty());

    ctx.send_meta(MetaUpdate {
        filename: Some(filename.clone()),
        url: Some(final_url.to_string()),
        total_bytes: total,
        resumable: Some(resumable),
        connections: Some(if use_multi { conns } else { 1 }),
        downloading: true,
        event: events.pop(),
        ..Default::default()
    });

    let outcome = if use_multi {
        drop(resp);
        download_segments(&mut ctx, &save_dir, &filename, total.unwrap(), conns, segments).await?
    } else {
        if !segments.is_empty() {
            ctx.db.clear_segments(&d.id)?;
        }
        let start = if is_partial { resume_from } else { 0 };
        download_single(&mut ctx, resp, &save_dir, &filename, start, total, read_timeout).await?
    };
    match outcome {
        EngineOutcome::Stopped => Ok(EngineOutcome::Stopped),
        EngineOutcome::Completed { .. } => finalize(&ctx, &save_dir, &filename, total).await,
    }
}

async fn download_single(
    ctx: &mut JobContext,
    resp: reqwest::Response,
    save_dir: &Path,
    filename: &str,
    start: u64,
    total: Option<u64>,
    read_timeout: Duration,
) -> Result<EngineOutcome> {
    let path = part_path(save_dir, filename);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .await
        .map_err(|e| DownloadError::fs("Cannot create file", &e))?;
    file.set_len(start).await.map_err(io_err)?;
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(io_err)?;
    let mut w = ChunkWriter::new(file);
    let mut stream = resp.bytes_stream();
    let mut meter = SpeedMeter::new(Duration::from_secs(4));
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut control = ctx.control.clone();

    loop {
        let next = tokio::select! {
            biased;
            _ = JobContext::stopped(&mut control) => {
                w.flush().await.map_err(io_err)?;
                return Ok(EngineOutcome::Stopped);
            }
            n = tokio::time::timeout(read_timeout, stream.next()) => n,
        };
        let chunk = match next {
            Err(_) => {
                w.flush().await.map_err(io_err)?;
                return Err(DownloadError::new(ErrorKind::Timeout, "Connection stalled").with_detail(format!("No data received for {}s", read_timeout.as_secs())));
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                w.flush().await.map_err(io_err)?;
                return Err(DownloadError::from_reqwest(&e));
            }
            Ok(Some(Ok(c))) => c,
        };
        if throttle(chunk.len() as u64, &ctx.download_limiter, &ctx.global_limiter, &mut control).await.is_some() {
            w.push(&chunk).await.map_err(io_err)?;
            w.flush().await.map_err(io_err)?;
            return Ok(EngineOutcome::Stopped);
        }
        w.push(&chunk).await.map_err(io_err)?;
        if let Some(t) = total {
            if start + w.committed + w.pending() > t {
                return Err(DownloadError::new(ErrorKind::ServerRejected, "Server sent more data than announced"));
            }
        }
        if last_report.elapsed() >= Duration::from_millis(250) {
            last_report = Instant::now();
            let done = start + w.committed + w.pending();
            let speed = meter.record(done);
            ctx.progress.set(EngineProgress { downloaded: done, total, speed_bps: speed, eta_seconds: eta(total, done, speed), ..Default::default() });
        }
    }
    w.flush().await.map_err(io_err)?;
    w.file.sync_all().await.map_err(io_err)?;
    let done = start + w.committed;
    ctx.progress.set(EngineProgress { downloaded: done, total, ..Default::default() });
    if let Some(t) = total {
        if done < t {
            return Err(DownloadError::new(ErrorKind::NetworkUnavailable, "Connection closed before the download finished")
                .with_detail(format!("Received {done} of {t} bytes")));
        }
    }
    Ok(EngineOutcome::Completed { filename: filename.to_string() })
}

/// Splits `total` bytes into at most `n` segments of at least MIN_SEGMENT.
pub fn plan_segments(total: u64, n: u32) -> Vec<Segment> {
    let n = (n as u64).min(total.div_ceil(MIN_SEGMENT)).max(1);
    let size = total.div_ceil(n);
    (0..n)
        .filter_map(|i| {
            let start = i * size;
            if start >= total {
                return None;
            }
            let end = ((i + 1) * size).min(total) - 1;
            Some(Segment { idx: i as u32, start, end_incl: end, downloaded: 0 })
        })
        .collect()
}

async fn download_segments(
    ctx: &mut JobContext,
    save_dir: &Path,
    filename: &str,
    total: u64,
    conns: u32,
    existing: Vec<Segment>,
) -> Result<EngineOutcome> {
    let id = ctx.download.id.clone();
    let path = part_path(save_dir, filename);
    let part_ok = tokio::fs::metadata(&path).await.map(|m| m.len() == total).unwrap_or(false);
    let valid_existing = !existing.is_empty()
        && part_ok
        && existing.iter().map(|s| s.len()).sum::<u64>() == total
        && existing.last().map(|s| s.end_incl) == Some(total - 1);
    let segments = if valid_existing {
        existing
    } else {
        let s = plan_segments(total, conns);
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .map_err(|e| DownloadError::fs("Cannot create file", &e))?;
        // Reserve the full size up front so parallel writes land at their offsets.
        f.set_len(total).await.map_err(io_err)?;
        f.sync_all().await.map_err(io_err)?;
        ctx.db.save_segments(&id, &s)?;
        s
    };

    let committed: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|s| AtomicU64::new(s.downloaded.min(s.len()))).collect());
    let inflight: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|_| AtomicU64::new(0)).collect());
    let (abort_tx, abort_rx) = watch::channel(Control::Run);
    let mut set = tokio::task::JoinSet::new();
    for (i, seg) in segments.iter().enumerate() {
        if seg.downloaded >= seg.len() {
            continue;
        }
        let seg = seg.clone();
        let params = SegmentParams {
            client: ctx.http.clone(),
            url: ctx.download.url.clone(),
            referrer: ctx.download.referrer.clone(),
            path: path.clone(),
            committed: committed.clone(),
            inflight: inflight.clone(),
            slot: i,
            per: ctx.download_limiter.clone(),
            global: ctx.global_limiter.clone(),
            control: ctx.control.clone(),
            abort: abort_rx.clone(),
            read_timeout: Duration::from_secs(ctx.settings.read_timeout_secs as u64),
        };
        set.spawn(run_segment(params, seg));
    }

    let mut meter = SpeedMeter::new(Duration::from_secs(4));
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut last_persist = Instant::now();
    let mut first_error: Option<DownloadError> = None;
    let mut stopped = false;

    let persist = |db: &crate::db::Db| {
        let prog: Vec<(u32, u64)> = segments.iter().enumerate().map(|(i, s)| (s.idx, committed[i].load(Ordering::Relaxed))).collect();
        let _ = db.update_segment_progress(&id, &prog);
    };

    loop {
        tokio::select! {
            res = set.join_next() => match res {
                None => break,
                Some(Ok(Ok(SegmentEnd::Done))) => {}
                Some(Ok(Ok(SegmentEnd::Stopped))) => stopped = true,
                Some(Ok(Err(e))) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                        let _ = abort_tx.send(Control::Pause);
                    }
                }
                Some(Err(join)) => {
                    if first_error.is_none() {
                        first_error = Some(DownloadError::new(ErrorKind::Unknown, "Download worker crashed").with_detail(join.to_string()));
                        let _ = abort_tx.send(Control::Pause);
                    }
                }
            },
            _ = tick.tick() => {
                let done: u64 = (0..segments.len()).map(|i| committed[i].load(Ordering::Relaxed) + inflight[i].load(Ordering::Relaxed)).sum();
                let speed = meter.record(done);
                ctx.progress.set(EngineProgress { downloaded: done, total: Some(total), speed_bps: speed, eta_seconds: eta(Some(total), done, speed), ..Default::default() });
                if last_persist.elapsed() >= Duration::from_secs(2) {
                    last_persist = Instant::now();
                    persist(&ctx.db);
                }
            }
        }
    }
    persist(&ctx.db);
    let done: u64 = (0..segments.len()).map(|i| committed[i].load(Ordering::Relaxed)).sum();
    ctx.progress.set(EngineProgress { downloaded: done, total: Some(total), ..Default::default() });
    if let Some(e) = first_error {
        return Err(e);
    }
    if stopped || ctx.control_state() != Control::Run {
        return Ok(EngineOutcome::Stopped);
    }
    if done != total {
        return Err(DownloadError::new(ErrorKind::NetworkUnavailable, "Download incomplete").with_detail(format!("{done} of {total} bytes")));
    }
    tokio::fs::OpenOptions::new().write(true).open(&path).await.map_err(io_err)?.sync_all().await.map_err(io_err)?;
    ctx.db.clear_segments(&id)?;
    Ok(EngineOutcome::Completed { filename: filename.to_string() })
}

struct SegmentParams {
    client: reqwest::Client,
    url: String,
    referrer: Option<String>,
    path: PathBuf,
    committed: Arc<Vec<AtomicU64>>,
    inflight: Arc<Vec<AtomicU64>>,
    slot: usize,
    per: Arc<RateLimiter>,
    global: Arc<RateLimiter>,
    control: watch::Receiver<Control>,
    abort: watch::Receiver<Control>,
    read_timeout: Duration,
}

enum SegmentEnd {
    Done,
    Stopped,
}

async fn run_segment(mut p: SegmentParams, seg: Segment) -> Result<SegmentEnd> {
    let mut attempt = 0;
    loop {
        match segment_attempt(&mut p, &seg).await {
            Ok(end) => return Ok(end),
            Err(e) if e.is_retryable() && attempt + 1 < SEGMENT_ATTEMPTS => {
                attempt += 1;
                tracing::debug!(segment = seg.idx, attempt, error = %e, "segment retry");
                let delay = Duration::from_millis(500 * 2u64.pow(attempt));
                let mut ctl = p.control.clone();
                let mut ab = p.abort.clone();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = JobContext::stopped(&mut ctl) => return Ok(SegmentEnd::Stopped),
                    _ = JobContext::stopped(&mut ab) => return Ok(SegmentEnd::Stopped),
                }
            }
            Err(e) => return Err(e),
        }
    }
}

async fn segment_attempt(p: &mut SegmentParams, seg: &Segment) -> Result<SegmentEnd> {
    let already = p.committed[p.slot].load(Ordering::Relaxed);
    if already >= seg.len() {
        return Ok(SegmentEnd::Done);
    }
    let from = seg.start + already;
    let req = request(&p.client, &p.url, p.referrer.as_deref()).header(header::RANGE, HeaderValue::from_str(&format!("bytes={from}-{}", seg.end_incl)).unwrap());
    let resp = tokio::select! {
        biased;
        _ = JobContext::stopped(&mut p.control) => return Ok(SegmentEnd::Stopped),
        _ = JobContext::stopped(&mut p.abort) => return Ok(SegmentEnd::Stopped),
        r = tokio::time::timeout(p.read_timeout * 2, req.send()) => match r {
            Err(_) => return Err(DownloadError::new(ErrorKind::Timeout, "Connection timed out")),
            Ok(r) => r.map_err(|e| DownloadError::from_reqwest(&e))?,
        },
    };
    let status = resp.status().as_u16();
    if status != 206 {
        if (200..300).contains(&status) {
            return Err(DownloadError::new(ErrorKind::ResumeNotSupported, "Server stopped honouring byte ranges"));
        }
        return Err(DownloadError::from_status(status));
    }
    if let Some((a, _)) = parse_content_range_span(resp.headers()) {
        if a != from {
            return Err(DownloadError::new(ErrorKind::ServerRejected, "Server returned the wrong byte range"));
        }
    }
    let mut file = tokio::fs::OpenOptions::new().write(true).open(&p.path).await.map_err(|e| DownloadError::fs("Cannot open file", &e))?;
    file.seek(std::io::SeekFrom::Start(from)).await.map_err(io_err)?;
    let mut w = ChunkWriter::new(file);
    let mut stream = resp.bytes_stream();
    let remaining = seg.len() - already;
    let mut received = 0u64;

    let commit = |w: &ChunkWriter, p: &SegmentParams| {
        p.committed[p.slot].store(already + w.committed, Ordering::Relaxed);
        p.inflight[p.slot].store(w.pending(), Ordering::Relaxed);
    };

    let end = loop {
        let next = tokio::select! {
            biased;
            _ = JobContext::stopped(&mut p.control) => break SegmentEnd::Stopped,
            _ = JobContext::stopped(&mut p.abort) => break SegmentEnd::Stopped,
            n = tokio::time::timeout(p.read_timeout, stream.next()) => n,
        };
        let chunk = match next {
            Err(_) => {
                w.flush().await.map_err(io_err)?;
                commit(&w, p);
                return Err(DownloadError::new(ErrorKind::Timeout, "Connection stalled"));
            }
            Ok(None) => break SegmentEnd::Done,
            Ok(Some(Err(e))) => {
                w.flush().await.map_err(io_err)?;
                commit(&w, p);
                return Err(DownloadError::from_reqwest(&e));
            }
            Ok(Some(Ok(c))) => c,
        };
        let take = (remaining - received).min(chunk.len() as u64) as usize;
        let mut ctl = p.control.clone();
        if throttle(take as u64, &p.per, &p.global, &mut ctl).await.is_some() {
            w.push(&chunk[..take]).await.map_err(io_err)?;
            break SegmentEnd::Stopped;
        }
        w.push(&chunk[..take]).await.map_err(io_err)?;
        received += take as u64;
        commit(&w, p);
        if received >= remaining {
            break SegmentEnd::Done;
        }
    };
    w.flush().await.map_err(io_err)?;
    commit(&w, p);
    match end {
        SegmentEnd::Done if received < remaining => Err(DownloadError::new(ErrorKind::NetworkUnavailable, "Connection closed early")),
        e => Ok(e),
    }
}

/// Atomically moves `<name>.part` to its final name.
async fn finalize(ctx: &JobContext, save_dir: &Path, filename: &str, total: Option<u64>) -> Result<EngineOutcome> {
    let part = part_path(save_dir, filename);
    let len = tokio::fs::metadata(&part).await.map_err(|e| DownloadError::fs("Partial file disappeared", &e))?.len();
    if let Some(t) = total {
        if len != t {
            return Err(DownloadError::new(ErrorKind::Filesystem, "Downloaded size does not match").with_detail(format!("{len} != {t}")));
        }
    }
    let mut final_name = filename.to_string();
    let target = save_dir.join(&final_name);
    if target.exists() {
        match ctx.settings.duplicate_policy {
            DuplicatePolicy::Overwrite => {
                tokio::fs::remove_file(&target).await.map_err(|e| DownloadError::fs("Cannot replace existing file", &e))?;
            }
            DuplicatePolicy::Rename => {
                final_name = security::resolve_duplicate(save_dir, filename, DuplicatePolicy::Rename, &|_| false);
            }
        }
    }
    tokio::fs::rename(&part, save_dir.join(&final_name)).await.map_err(|e| DownloadError::fs("Cannot finalize file", &e))?;
    ctx.progress.set(EngineProgress { downloaded: len, total: Some(len), ..Default::default() });
    Ok(EngineOutcome::Completed { filename: final_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_planning() {
        let s = plan_segments(10 * 1024 * 1024, 4);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].start, 0);
        assert_eq!(s.last().unwrap().end_incl, 10 * 1024 * 1024 - 1);
        assert_eq!(s.iter().map(|x| x.len()).sum::<u64>(), 10 * 1024 * 1024);
        for w in s.windows(2) {
            assert_eq!(w[0].end_incl + 1, w[1].start);
        }
        // Small files never get more segments than MIN_SEGMENT allows.
        assert_eq!(plan_segments(MIN_SEGMENT + 1, 16).len(), 2);
        assert_eq!(plan_segments(1, 8).len(), 1);
        let odd = plan_segments(3_000_001, 7);
        assert_eq!(odd.iter().map(|x| x.len()).sum::<u64>(), 3_000_001);
    }

    #[test]
    fn content_range() {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes 100-199/1000"));
        assert_eq!(parse_content_range_total(&h), Some(1000));
        assert_eq!(parse_content_range_span(&h), Some((100, 199)));
        h.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes */1000"));
        assert_eq!(parse_content_range_total(&h), Some(1000));
    }

    #[test]
    fn filename_detection() {
        let u = url::Url::parse("https://example.com/download?id=5").unwrap();
        let mut h = HeaderMap::new();
        assert_eq!(detect_filename(&h, &u, Some("application/pdf")).as_deref(), Some("download.pdf"));
        h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"report 2024.xlsx\""));
        assert_eq!(detect_filename(&h, &u, None).as_deref(), Some("report 2024.xlsx"));
    }

    #[test]
    fn weak_etags_not_used() {
        assert!(!usable_etag("W/\"abc\""));
        assert!(usable_etag("\"abc\""));
    }
}
