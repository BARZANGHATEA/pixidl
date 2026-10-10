//! HTTP/HTTPS engine.
//!
//! - streams to `<name>.part`, never buffering whole files in memory
//! - resumes with `Range` + `If-Range` (ETag / Last-Modified) so a changed
//!   resource is detected instead of silently corrupting the file
//! - detects servers without range support and restarts honestly
//! - splits large resumable files into segments downloaded over parallel
//!   connections with work stealing: a connection that runs out of work
//!   takes half of the biggest remaining range, so every connection stays
//!   busy until the end; segment progress is persisted so a crash can resume
//! - tolerates servers that limit connections per client: a refused extra
//!   connection hands its range back and the download carries on with fewer
//! - enforces the per-download and global rate limits
//! - renames `.part` → final name only after the byte count is verified

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderValue};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;

use super::{available_space, eta, Control, Engine, EngineOutcome, EngineProgress, JobContext, MetaUpdate, SegmentInfo, SegmentView, SpeedMeter};
use crate::db::{Segment, Validators};
use crate::error::{DownloadError, Result};
use crate::ratelimit::RateLimiter;
use crate::security::{self, DuplicatePolicy};
use crate::settings::{ProxyMode, Settings, MAX_CONNECTIONS};
use crate::types::{Download, EngineKind, ErrorKind};

/// Files smaller than this are always downloaded over one connection.
const MULTI_CONNECTION_THRESHOLD: u64 = 2 * 1024 * 1024;
/// Smallest segment of the initial split.
const MIN_SEGMENT: u64 = 512 * 1024;
/// A range is only split for an idle connection when both halves get at
/// least this much; below that a new request costs more than it saves.
pub const MIN_SPLIT: u64 = 512 * 1024;
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
        .pool_max_idle_per_host(MAX_CONNECTIONS as usize)
        // One TCP connection per segment: over HTTP/2 every range request
        // would be multiplexed onto a single connection, which defeats
        // multi-connection downloads (servers throttle per connection).
        .http1_only()
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
    let mut segments = ctx.db.load_segments(&d.id)?;
    segments.sort_by_key(|s| s.start);
    let part_len = tokio::fs::metadata(part_path(&save_dir, &filename)).await.map(|m| m.len()).unwrap_or(0);
    let validators = ctx.db.validators(&d.id)?;
    // Single-stream resume continues at the end of the partial file.
    let resume_from = if segments.is_empty() { part_len } else { 0 };
    // A segmented resume asks for the first missing byte, so this response
    // can be handed straight to the worker of that range.
    let seg_total = segments.last().map(|s| s.end_incl + 1);
    let request_from = if segments.is_empty() { part_len } else { segments.iter().find(|s| !s.is_done()).map(|s| s.start + s.downloaded).unwrap_or(0) };

    let mut req = request(&ctx.http, &d.url, d.referrer.as_deref()).header(header::RANGE, format!("bytes={request_from}-"));
    let has_validator = validators.etag.as_deref().is_some_and(usable_etag) || validators.last_modified.is_some();
    if request_from > 0 || !segments.is_empty() {
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

    if status == 416 && request_from > 0 {
        // Range not satisfiable: either the file is already complete or it changed.
        if segments.is_empty() && parse_content_range_total(&headers) == Some(resume_from) {
            drop(resp);
            return finalize(&ctx, &save_dir, &filename, Some(resume_from)).await;
        }
        drop(resp);
        let _ = ctx.db.clear_segments(&d.id);
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
    let total = if is_partial { parse_content_range_total(&headers).or(seg_total) } else { resp.content_length() };
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
        let already = match (is_partial, segments.is_empty()) {
            (false, _) => 0,
            (true, true) => resume_from,
            (true, false) => segments.iter().map(|s| s.downloaded.min(s.len())).sum(),
        };
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
    let conns = d.engine_options.connections.unwrap_or(ctx.settings.connections_per_download).clamp(1, MAX_CONNECTIONS);
    // Persisted segments are always continued segment-wise (even with one
    // connection now): the partial file has holes a single stream can't fill.
    let use_multi = is_partial
        && total.is_some()
        && (!segments.is_empty() || (conns > 1 && resume_from == 0 && total.is_some_and(|t| t >= MULTI_CONNECTION_THRESHOLD)));

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
        let starts_at = parse_content_range_span(&headers).map(|(a, _)| a);
        let initial = (starts_at == Some(request_from)).then_some((request_from, resp));
        download_segments(&mut ctx, &save_dir, &filename, total.unwrap(), conns, segments, initial).await?
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

/// Whether persisted segments describe a usable layout for a `total`-byte
/// file: sorted, contiguous and covering every byte exactly once.
pub fn segments_cover(segs: &[Segment], total: u64) -> bool {
    !segs.is_empty()
        && total > 0
        && segs[0].start == 0
        && segs.iter().all(|s| s.end_incl >= s.start)
        && segs.windows(2).all(|w| w[0].end_incl + 1 == w[1].start)
        && segs.last().map(|s| s.end_incl) == Some(total - 1)
}

/// Folds every finished segment into the one that follows it (their bytes
/// are contiguous on disk) so splits from earlier runs don't pile up, and
/// renumbers the result. Expects segments sorted by `start`.
pub fn compact_segments(segs: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
    for mut s in segs {
        s.downloaded = s.downloaded.min(s.len());
        if let Some(prev) = out.last_mut() {
            if prev.is_done() {
                prev.downloaded = prev.len() + s.downloaded;
                prev.end_incl = s.end_incl;
                continue;
            }
        }
        out.push(s);
    }
    for (i, s) in out.iter_mut().enumerate() {
        s.idx = i as u32;
    }
    out
}

/// One byte range in the live segment table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegSlot {
    pub idx: u32,
    pub start: u64,
    /// Exclusive. Only ever moves left (when another worker steals the tail).
    pub end: u64,
    /// Bytes accepted from the network, possibly still in a write buffer.
    pub received: u64,
    /// Bytes handed to the OS. Only these are persisted, so after a crash
    /// anything past `written` is downloaded again.
    pub written: u64,
    /// A worker owns this range.
    pub active: bool,
}

impl SegSlot {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
    /// Next byte the owner will receive.
    pub fn pos(&self) -> u64 {
        self.start + self.received
    }
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos())
    }
    pub fn is_done(&self) -> bool {
        self.written >= self.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquire {
    /// Work on this slot.
    Slot(usize),
    /// Everything left is owned by other workers and too small to split.
    Wait,
    /// Every byte is on disk.
    AllDone,
}

/// The work-stealing segment table shared by the workers of one download.
///
/// A worker takes the lowest unowned unfinished range; when there is none it
/// splits the active range with the most bytes left at the midpoint of what
/// remains, shrinking the victim's `end` and taking the upper half. Workers
/// claim bytes through [`SegmentTable::claim`] under the same lock, so a
/// victim can never write past its new end. Slots are only appended, so an
/// index stays valid for the whole run.
#[derive(Debug, Clone)]
pub struct SegmentTable {
    pub slots: Vec<SegSlot>,
    min_split: u64,
    next_idx: u32,
    /// The layout changed since it was last persisted.
    pub dirty: bool,
}

impl SegmentTable {
    pub fn new(segs: &[Segment], min_split: u64) -> Self {
        let slots: Vec<SegSlot> = segs
            .iter()
            .map(|s| {
                let w = s.downloaded.min(s.len());
                SegSlot { idx: s.idx, start: s.start, end: s.end_incl + 1, received: w, written: w, active: false }
            })
            .collect();
        let next_idx = slots.iter().map(|s| s.idx + 1).max().unwrap_or(0);
        Self { slots, min_split: min_split.max(1), next_idx, dirty: false }
    }

    pub fn acquire(&mut self) -> Acquire {
        if let Some(i) = self.slots.iter().enumerate().filter(|(_, s)| !s.active && !s.is_done()).min_by_key(|(_, s)| s.start).map(|(i, _)| i) {
            let s = &mut self.slots[i];
            s.received = s.written;
            s.active = true;
            return Acquire::Slot(i);
        }
        if self.slots.iter().all(|s| s.is_done()) {
            return Acquire::AllDone;
        }
        match self.split_point() {
            Some((victim, mid)) => {
                let old_end = self.slots[victim].end;
                self.slots[victim].end = mid;
                self.slots.push(SegSlot { idx: self.next_idx, start: mid, end: old_end, received: 0, written: 0, active: true });
                self.next_idx += 1;
                self.dirty = true;
                Acquire::Slot(self.slots.len() - 1)
            }
            None => Acquire::Wait,
        }
    }

    /// The active range with the most bytes left and where to cut it, if
    /// both halves would get at least `min_split` bytes.
    pub fn split_point(&self) -> Option<(usize, u64)> {
        let (i, s) = self.slots.iter().enumerate().filter(|(_, s)| s.active).max_by_key(|(_, s)| s.remaining())?;
        let rem = s.remaining();
        if rem < 2 * self.min_split {
            return None;
        }
        Some((i, s.pos() + rem / 2))
    }

    /// Accepts up to `n` more bytes for slot `i`. Returns how many may be
    /// written (fewer when the range was shrunk) and whether the range is
    /// now fully received.
    pub fn claim(&mut self, i: usize, n: u64) -> (u64, bool) {
        let s = &mut self.slots[i];
        let take = n.min(s.remaining());
        s.received += take;
        (take, s.pos() >= s.end)
    }

    pub fn set_written(&mut self, i: usize, written: u64) {
        let s = &mut self.slots[i];
        s.written = written.min(s.len());
    }

    /// Gives the range back; bytes not yet on disk will be fetched again.
    pub fn release(&mut self, i: usize) {
        let s = &mut self.slots[i];
        s.received = s.written;
        s.active = false;
    }

    pub fn complete(&self) -> bool {
        self.slots.iter().all(|s| s.is_done())
    }

    pub fn written_bytes(&self) -> u64 {
        self.slots.iter().map(|s| s.written).sum()
    }

    pub fn received_bytes(&self) -> u64 {
        self.slots.iter().map(|s| s.received.min(s.len())).sum()
    }

    /// Persistable layout, sorted by offset.
    pub fn snapshot(&self) -> Vec<Segment> {
        let mut v: Vec<Segment> =
            self.slots.iter().filter(|s| !s.is_empty()).map(|s| Segment { idx: s.idx, start: s.start, end_incl: s.end - 1, downloaded: s.written }).collect();
        v.sort_by_key(|s| s.start);
        v
    }

    pub fn view(&self, total: u64, connections: u32) -> SegmentView {
        let mut segments: Vec<SegmentInfo> = self
            .slots
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| SegmentInfo { start: s.start, end: s.end, downloaded: s.received.min(s.len()), active: s.active && !s.is_done() })
            .collect();
        segments.sort_by_key(|s| s.start);
        SegmentView { total, segments, connections }
    }
}

/// UI view of segments persisted by a paused/interrupted download.
pub fn saved_segment_view(total: u64, segs: &[Segment]) -> SegmentView {
    let mut segments: Vec<SegmentInfo> =
        segs.iter().map(|s| SegmentInfo { start: s.start, end: s.end_incl + 1, downloaded: s.downloaded.min(s.len()), active: false }).collect();
    segments.sort_by_key(|s| s.start);
    SegmentView { total, segments, connections: 0 }
}

/// Counts a connection that is receiving data while it lives.
struct StreamGuard(Arc<AtomicU32>);

impl StreamGuard {
    fn new(c: &Arc<AtomicU32>) -> Self {
        c.fetch_add(1, Ordering::SeqCst);
        Self(c.clone())
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The response of the engine's first request, handed to whichever worker
/// starts at `offset` instead of opening another connection.
struct InitialResponse {
    offset: u64,
    resp: reqwest::Response,
    guard: StreamGuard,
}

struct Shared {
    table: parking_lot::Mutex<SegmentTable>,
    total: u64,
    client: reqwest::Client,
    url: String,
    referrer: Option<String>,
    path: PathBuf,
    per: Arc<RateLimiter>,
    global: Arc<RateLimiter>,
    read_timeout: Duration,
    /// Workers currently holding an open, data-producing connection.
    streaming: Arc<AtomicU32>,
    initial: parking_lot::Mutex<Option<InitialResponse>>,
}

enum WorkerEnd {
    /// No work left.
    Finished,
    Stopped,
    /// The server refused another connection while others were working;
    /// this worker gave its range back and quit.
    Limited(DownloadError),
    /// Retries exhausted; the range was given back for other workers.
    Failed(DownloadError),
    /// The download cannot continue at all.
    Fatal(DownloadError),
}

enum SlotEnd {
    Done,
    Stopped,
    Limited(DownloadError),
    Failed(DownloadError),
    Fatal(DownloadError),
}

enum AttemptEnd {
    Done,
    Stopped,
}

enum AttemptErr {
    /// Rejected before any data: 429/503/403, refused or reset connection,
    /// or a range request answered with the whole file.
    Refused(DownloadError),
    Failed(DownloadError),
    Fatal(DownloadError),
}

async fn download_segments(
    ctx: &mut JobContext,
    save_dir: &Path,
    filename: &str,
    total: u64,
    conns: u32,
    existing: Vec<Segment>,
    initial: Option<(u64, reqwest::Response)>,
) -> Result<EngineOutcome> {
    let id = ctx.download.id.clone();
    let path = part_path(save_dir, filename);
    let part_ok = tokio::fs::metadata(&path).await.map(|m| m.len() == total).unwrap_or(false);
    let segments = if part_ok && segments_cover(&existing, total) {
        compact_segments(existing)
    } else {
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
        plan_segments(total, conns)
    };
    ctx.db.save_segments(&id, &segments)?;
    let table = SegmentTable::new(&segments, MIN_SPLIT);
    let left = total - table.written_bytes();
    // No point in more workers than there are MIN_SPLIT-sized pieces left.
    let workers = (conns as u64).min(left.div_ceil(MIN_SPLIT)).max(1) as u32;

    let streaming = Arc::new(AtomicU32::new(0));
    // Use the first response only if a range starts exactly where it does.
    let initial = initial.and_then(|(offset, resp)| {
        let usable = table.slots.iter().any(|s| !s.is_done() && s.pos() == offset);
        usable.then(|| InitialResponse { offset, resp, guard: StreamGuard::new(&streaming) })
    });
    let sh = Arc::new(Shared {
        table: parking_lot::Mutex::new(table),
        total,
        client: ctx.http.clone(),
        url: ctx.download.url.clone(),
        referrer: ctx.download.referrer.clone(),
        path: path.clone(),
        per: ctx.download_limiter.clone(),
        global: ctx.global_limiter.clone(),
        read_timeout: Duration::from_secs(ctx.settings.read_timeout_secs as u64),
        streaming: streaming.clone(),
        initial: parking_lot::Mutex::new(initial),
    });

    let (abort_tx, abort_rx) = watch::channel(Control::Run);
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..workers {
        set.spawn(worker(sh.clone(), ctx.control.clone(), abort_rx.clone()));
    }
    if workers != conns {
        ctx.send_meta(MetaUpdate { connections: Some(workers), ..Default::default() });
    }

    let mut meter = SpeedMeter::new(Duration::from_secs(4));
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut last_persist = Instant::now();
    let mut fatal: Option<DownloadError> = None;
    let mut last_error: Option<DownloadError> = None;
    let mut stopped = false;
    // Set when a worker quit because of a connection limit; reported once
    // things have settled.
    let mut limited_at: Option<Instant> = None;
    let mut limit_reported = false;

    let persist = |db: &crate::db::Db, sh: &Shared| {
        let snap = {
            let mut t = sh.table.lock();
            t.dirty = false;
            t.snapshot()
        };
        if let Err(e) = db.save_segments(&id, &snap) {
            tracing::warn!(error = %e, "cannot persist segments");
        }
    };

    loop {
        tokio::select! {
            res = set.join_next() => match res {
                None => break,
                Some(Ok(WorkerEnd::Finished)) => {}
                Some(Ok(WorkerEnd::Stopped)) => stopped = true,
                Some(Ok(WorkerEnd::Limited(e))) => {
                    tracing::debug!(error = %e, left = set.len(), "connection refused by server; continuing with fewer connections");
                    limited_at = Some(Instant::now());
                }
                Some(Ok(WorkerEnd::Failed(e))) => {
                    tracing::debug!(error = %e, left = set.len(), "segment worker gave up");
                    last_error = Some(e);
                }
                Some(Ok(WorkerEnd::Fatal(e))) => {
                    if fatal.is_none() {
                        fatal = Some(e);
                        let _ = abort_tx.send(Control::Pause);
                    }
                }
                Some(Err(join)) => {
                    if fatal.is_none() {
                        fatal = Some(DownloadError::new(ErrorKind::Unknown, "Download worker crashed").with_detail(join.to_string()));
                        let _ = abort_tx.send(Control::Pause);
                    }
                }
            },
            _ = tick.tick() => {
                let (done, view, dirty) = {
                    let t = sh.table.lock();
                    (t.received_bytes(), t.view(total, sh.streaming.load(Ordering::SeqCst)), t.dirty)
                };
                let speed = meter.record(done);
                ctx.progress.set(EngineProgress { downloaded: done, total: Some(total), speed_bps: speed, eta_seconds: eta(Some(total), done, speed), ..Default::default() });
                ctx.progress.set_segments(Some(view));
                if dirty || last_persist.elapsed() >= Duration::from_secs(2) {
                    last_persist = Instant::now();
                    persist(&ctx.db, &sh);
                }
                if let Some(at) = limited_at {
                    if at.elapsed() >= Duration::from_secs(1) && fatal.is_none() && !set.is_empty() {
                        limited_at = None;
                        let n = set.len() as u32;
                        ctx.send_meta(MetaUpdate {
                            connections: Some(n),
                            event: (!limit_reported).then(|| format!("Server allows only {n} connection{}; continuing with fewer", if n == 1 { "" } else { "s" })),
                            ..Default::default()
                        });
                        limit_reported = true;
                    }
                }
            }
        }
    }
    persist(&ctx.db, &sh);
    let (done, complete, view) = {
        let t = sh.table.lock();
        (t.written_bytes(), t.complete(), t.view(total, 0))
    };
    ctx.progress.set(EngineProgress { downloaded: done, total: Some(total), ..Default::default() });
    ctx.progress.set_segments(Some(view));
    if let Some(e) = fatal {
        return Err(e);
    }
    if stopped || ctx.control_state() != Control::Run {
        return Ok(EngineOutcome::Stopped);
    }
    if !complete {
        return Err(last_error
            .unwrap_or_else(|| DownloadError::new(ErrorKind::NetworkUnavailable, "Download incomplete"))
            .with_detail(format!("{done} of {total} bytes received")));
    }
    tokio::fs::OpenOptions::new().write(true).open(&path).await.map_err(io_err)?.sync_all().await.map_err(io_err)?;
    ctx.db.clear_segments(&id)?;
    Ok(EngineOutcome::Completed { filename: filename.to_string() })
}

fn halted(c: &watch::Receiver<Control>) -> bool {
    *c.borrow() != Control::Run
}

/// One connection's worth of work: takes ranges (or steals half of one)
/// until everything is on disk.
async fn worker(sh: Arc<Shared>, mut control: watch::Receiver<Control>, mut abort: watch::Receiver<Control>) -> WorkerEnd {
    loop {
        if halted(&control) || halted(&abort) {
            return WorkerEnd::Stopped;
        }
        let next = sh.table.lock().acquire();
        match next {
            Acquire::AllDone => return WorkerEnd::Finished,
            Acquire::Wait => {
                // Another worker may give a range back (error, limit) — look again shortly.
                tokio::select! {
                    _ = JobContext::stopped(&mut control) => return WorkerEnd::Stopped,
                    _ = JobContext::stopped(&mut abort) => return WorkerEnd::Stopped,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
            Acquire::Slot(i) => {
                let end = run_slot(&sh, i, &mut control, &mut abort).await;
                sh.table.lock().release(i);
                match end {
                    SlotEnd::Done => {}
                    SlotEnd::Stopped => return WorkerEnd::Stopped,
                    SlotEnd::Limited(e) => return WorkerEnd::Limited(e),
                    SlotEnd::Failed(e) => return WorkerEnd::Failed(e),
                    SlotEnd::Fatal(e) => return WorkerEnd::Fatal(e),
                }
            }
        }
    }
}

/// Downloads slot `i` to its (possibly shrinking) end, retrying transient errors.
async fn run_slot(sh: &Shared, i: usize, control: &mut watch::Receiver<Control>, abort: &mut watch::Receiver<Control>) -> SlotEnd {
    let mut failures = 0;
    loop {
        let before = sh.table.lock().slots[i].written;
        let res = slot_attempt(sh, i, control, abort).await;
        let e = match res {
            Ok(AttemptEnd::Done) => return SlotEnd::Done,
            Ok(AttemptEnd::Stopped) => return SlotEnd::Stopped,
            Err(AttemptErr::Fatal(e)) => return SlotEnd::Fatal(e),
            // Others are getting data: the server is limiting connections, so
            // give the range back instead of hammering it.
            Err(AttemptErr::Refused(e)) if sh.streaming.load(Ordering::SeqCst) > 0 => return SlotEnd::Limited(e),
            Err(AttemptErr::Refused(e)) | Err(AttemptErr::Failed(e)) => e,
        };
        if sh.table.lock().slots[i].written > before {
            failures = 0; // made progress: a fresh budget for the next hiccup
        }
        failures += 1;
        if !e.is_retryable() || failures >= SEGMENT_ATTEMPTS {
            return SlotEnd::Failed(e);
        }
        tracing::debug!(slot = i, failures, error = %e, "segment retry");
        let delay = Duration::from_millis(500 * 2u64.pow(failures));
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = JobContext::stopped(control) => return SlotEnd::Stopped,
            _ = JobContext::stopped(abort) => return SlotEnd::Stopped,
        }
    }
}

async fn slot_attempt(sh: &Shared, i: usize, control: &mut watch::Receiver<Control>, abort: &mut watch::Receiver<Control>) -> Result<AttemptEnd, AttemptErr> {
    let (from, end, base) = {
        let mut t = sh.table.lock();
        let s = &mut t.slots[i];
        s.received = s.written;
        (s.pos(), s.end, s.written)
    };
    if from >= end {
        return Ok(AttemptEnd::Done);
    }
    let initial = {
        let mut g = sh.initial.lock();
        if g.as_ref().is_some_and(|r| r.offset == from) {
            g.take()
        } else {
            None
        }
    };
    let (resp, _guard) = match initial {
        Some(r) => (r.resp, r.guard),
        None => {
            let range = HeaderValue::from_str(&format!("bytes={from}-{}", end - 1)).unwrap();
            let req = request(&sh.client, &sh.url, sh.referrer.as_deref()).header(header::RANGE, range);
            let resp = tokio::select! {
                biased;
                _ = JobContext::stopped(control) => return Ok(AttemptEnd::Stopped),
                _ = JobContext::stopped(abort) => return Ok(AttemptEnd::Stopped),
                r = tokio::time::timeout(sh.read_timeout * 2, req.send()) => match r {
                    Err(_) => return Err(AttemptErr::Failed(DownloadError::new(ErrorKind::Timeout, "Connection timed out"))),
                    Ok(Err(e)) => {
                        let de = DownloadError::from_reqwest(&e);
                        return Err(if e.is_connect() || e.is_request() { AttemptErr::Refused(de) } else { AttemptErr::Failed(de) });
                    }
                    Ok(Ok(r)) => r,
                },
            };
            let status = resp.status().as_u16();
            match status {
                206 => {}
                403 | 429 | 503 => return Err(AttemptErr::Refused(DownloadError::from_status(status))),
                200..=299 => {
                    return Err(AttemptErr::Refused(
                        DownloadError::new(ErrorKind::ResumeNotSupported, "Server stopped honouring byte ranges").with_detail(format!("HTTP status {status}")),
                    ))
                }
                _ => return Err(AttemptErr::Failed(DownloadError::from_status(status))),
            }
            let span = parse_content_range_span(resp.headers());
            if span.is_some_and(|(a, _)| a != from) {
                return Err(AttemptErr::Fatal(DownloadError::new(ErrorKind::ServerRejected, "Server returned the wrong byte range")));
            }
            if parse_content_range_total(resp.headers()).is_some_and(|t| t != sh.total) {
                return Err(AttemptErr::Fatal(
                    DownloadError::new(ErrorKind::ResumeNotSupported, "The file on the server changed during the download").with_detail("Content-Range total differs"),
                ));
            }
            let guard = StreamGuard::new(&sh.streaming);
            (resp, guard)
        }
    };

    let fatal_io = |e: std::io::Error| AttemptErr::Fatal(DownloadError::from_io(&e));
    let mut file = tokio::fs::OpenOptions::new().write(true).open(&sh.path).await.map_err(|e| AttemptErr::Fatal(DownloadError::fs("Cannot open file", &e)))?;
    file.seek(std::io::SeekFrom::Start(from)).await.map_err(fatal_io)?;
    let mut w = ChunkWriter::new(file);
    let mut stream = resp.bytes_stream();
    let mut got_any = false;

    let end = loop {
        let next = tokio::select! {
            biased;
            _ = JobContext::stopped(control) => break Ok(AttemptEnd::Stopped),
            _ = JobContext::stopped(abort) => break Ok(AttemptEnd::Stopped),
            n = tokio::time::timeout(sh.read_timeout, stream.next()) => n,
        };
        let chunk = match next {
            Err(_) => break Err(AttemptErr::Failed(DownloadError::new(ErrorKind::Timeout, "Connection stalled"))),
            Ok(None) => break Err(AttemptErr::Failed(DownloadError::new(ErrorKind::NetworkUnavailable, "Connection closed early"))),
            Ok(Some(Err(e))) => {
                let de = DownloadError::from_reqwest(&e);
                break Err(if got_any { AttemptErr::Failed(de) } else { AttemptErr::Refused(de) });
            }
            Ok(Some(Ok(c))) => c,
        };
        let want = sh.table.lock().slots[i].remaining().min(chunk.len() as u64);
        if throttle(want, &sh.per, &sh.global, control).await.is_some() {
            break Ok(AttemptEnd::Stopped);
        }
        // Claim under the table lock: a concurrent split may have moved our end.
        let (take, reached) = sh.table.lock().claim(i, want);
        if let Err(e) = w.push(&chunk[..take as usize]).await {
            break Err(fatal_io(e));
        }
        got_any = true;
        sh.table.lock().set_written(i, base + w.committed);
        if reached {
            // Closing the response here drops the connection; the rest of
            // the server's stream belongs to another range.
            break Ok(AttemptEnd::Done);
        }
    };
    drop(stream);
    w.flush().await.map_err(fatal_io)?;
    sh.table.lock().set_written(i, base + w.committed);
    let end = end?;
    if matches!(end, AttemptEnd::Done) && !sh.table.lock().slots[i].is_done() {
        return Err(AttemptErr::Failed(DownloadError::new(ErrorKind::NetworkUnavailable, "Connection closed early")));
    }
    Ok(end)
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

    const M: u64 = MIN_SPLIT;

    fn table(total: u64, n: u32) -> SegmentTable {
        SegmentTable::new(&plan_segments(total, n), M)
    }

    #[test]
    fn workers_take_unstarted_ranges_first() {
        let mut t = table(8 * M, 4);
        let got: Vec<_> = (0..4).map(|_| t.acquire()).collect();
        assert_eq!(got, vec![Acquire::Slot(0), Acquire::Slot(1), Acquire::Slot(2), Acquire::Slot(3)]);
        assert!(t.slots.iter().all(|s| s.active));
        assert!(!t.dirty, "taking planned ranges does not change the layout");
    }

    #[test]
    fn idle_worker_splits_largest_remaining_at_midpoint() {
        let mut t = table(8 * M, 2); // two ranges of 4M
        assert_eq!(t.acquire(), Acquire::Slot(0));
        assert_eq!(t.acquire(), Acquire::Slot(1));
        // Range 0 is 1M in, range 1 is 3M in.
        t.claim(0, M);
        t.claim(1, 3 * M);
        assert_eq!(t.split_point(), Some((0, M + (3 * M) / 2)));
        assert_eq!(t.acquire(), Acquire::Slot(2));
        assert!(t.dirty);
        assert_eq!(t.slots[0].end, M + 3 * M / 2);
        assert_eq!(t.slots[2].start, M + 3 * M / 2);
        assert_eq!(t.slots[2].end, 4 * M);
        assert_eq!(t.slots[2].idx, 2);
        let snap = t.snapshot();
        assert!(segments_cover(&snap, 8 * M), "{snap:?}");
        assert_eq!(snap.iter().map(|s| s.start).collect::<Vec<_>>(), vec![0, M + 3 * M / 2, 4 * M]);
    }

    #[test]
    fn small_remainders_are_not_split() {
        let mut t = table(4 * M, 1);
        assert_eq!(t.acquire(), Acquire::Slot(0));
        t.claim(0, 4 * M - 2 * M + 1); // 2M - 1 left
        assert_eq!(t.acquire(), Acquire::Wait);
        t.claim(0, 4 * M);
        t.set_written(0, 4 * M);
        t.release(0);
        assert_eq!(t.acquire(), Acquire::AllDone);
        assert!(t.complete());
    }

    #[test]
    fn victim_never_claims_past_its_new_end() {
        let mut t = table(4 * M, 1);
        assert_eq!(t.acquire(), Acquire::Slot(0));
        t.claim(0, 1000);
        assert_eq!(t.acquire(), Acquire::Slot(1));
        let mid = t.slots[0].end;
        assert_eq!(mid, 1000 + (4 * M - 1000) / 2);
        // The victim's stream still delivers everything up to the old end.
        let (take, reached) = t.claim(0, 4 * M);
        assert_eq!(take, mid - 1000);
        assert!(reached);
        assert_eq!(t.claim(0, 10), (0, true));
        assert_eq!(t.slots[0].pos(), mid);
    }

    #[test]
    fn release_forgets_unwritten_bytes() {
        let mut t = table(4 * M, 1);
        assert_eq!(t.acquire(), Acquire::Slot(0));
        t.claim(0, 5000);
        t.set_written(0, 4096);
        t.release(0);
        assert_eq!(t.slots[0].received, 4096);
        assert!(!t.slots[0].active);
        // The next worker continues from what is on disk.
        assert_eq!(t.acquire(), Acquire::Slot(0));
        assert_eq!(t.slots[0].pos(), 4096);
        assert_eq!(t.snapshot()[0].downloaded, 4096);
    }

    #[test]
    fn compaction_merges_finished_ranges() {
        let segs = vec![
            Segment { idx: 0, start: 0, end_incl: 99, downloaded: 100 },
            Segment { idx: 3, start: 100, end_incl: 199, downloaded: 100 },
            Segment { idx: 1, start: 200, end_incl: 299, downloaded: 40 },
            Segment { idx: 2, start: 300, end_incl: 399, downloaded: 100 },
            Segment { idx: 4, start: 400, end_incl: 499, downloaded: 7 },
        ];
        let c = compact_segments(segs);
        assert_eq!(
            c,
            vec![
                Segment { idx: 0, start: 0, end_incl: 299, downloaded: 240 },
                Segment { idx: 1, start: 300, end_incl: 499, downloaded: 107 },
            ]
        );
        assert!(segments_cover(&c, 500));
        assert!(!segments_cover(&c, 600));
        assert!(!segments_cover(&c[1..], 500));
    }

    /// Random interleaving of many workers: every byte is claimed exactly
    /// once, the layout always covers the file, and work is spread to the end.
    #[test]
    fn simulated_work_stealing_claims_every_byte_once() {
        use rand::{Rng, SeedableRng};
        let total = 37 * M + 12_345;
        for seed in 0..20u64 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut t = table(total, 6);
            let mut owner: Vec<Option<usize>> = vec![None; 8];
            let mut seen = vec![0u8; total as usize];
            loop {
                let w = rng.gen_range(0..owner.len());
                match owner[w] {
                    None => match t.acquire() {
                        Acquire::Slot(i) => owner[w] = Some(i),
                        Acquire::Wait => {}
                        Acquire::AllDone => break,
                    },
                    Some(i) => {
                        let pos = t.slots[i].pos();
                        let (take, reached) = t.claim(i, rng.gen_range(1..300_000));
                        for b in &mut seen[pos as usize..(pos + take) as usize] {
                            *b += 1;
                        }
                        let rec = t.slots[i].received;
                        t.set_written(i, rec);
                        if reached {
                            t.release(i);
                            owner[w] = None;
                        }
                    }
                }
                assert!(segments_cover(&t.snapshot(), total));
            }
            assert!(seen.iter().all(|&b| b == 1), "seed {seed}: a byte was claimed {} times", seen.iter().find(|&&b| b != 1).unwrap());
            assert!(t.complete());
            assert!(t.slots.len() > 6, "idle workers must steal work");
            assert!(t.slots.iter().all(|s| s.is_empty() || s.len() >= M), "no range smaller than MIN_SPLIT was created");
        }
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
