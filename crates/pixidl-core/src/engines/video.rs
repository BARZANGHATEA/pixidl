//! Video/site engine backed by yt-dlp, run as an isolated child process.
//!
//! - arguments are passed as an array; the URL is validated and placed after
//!   `--` so it can never be interpreted as an option
//! - user yt-dlp config files are ignored (`--ignore-config`) for predictable behaviour
//! - partial data lives in `<save_dir>/.pixidl-partial/<id>` and is moved into
//!   place by yt-dlp only when complete; cancel removes that directory
//! - pause = stop the process; resume = re-run (yt-dlp continues `.part` files)

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use ts_rs::TS;

use super::{eta, Engine, EngineOutcome, EngineProgress, JobContext, MetaUpdate};
use crate::error::{redact, DownloadError, Result};
use crate::security;
use crate::settings::{ProxyMode, Settings};
use crate::tools::{self, command, ToolLocator};
use crate::types::{Download, EngineKind, ErrorKind, ToolSetup, ToolSetupPhase};

const PROGRESS_PREFIX: &str = "PIXIDLPROG|";
const META_PREFIX: &str = "PIXIDLMETA|";
const FILE_PREFIX: &str = "PIXIDLFILE|";
const INSPECT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoFormat {
    pub format_id: String,
    pub ext: String,
    pub resolution: Option<String>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    #[ts(type = "number | null")]
    pub filesize: Option<u64>,
    pub tbr: Option<f64>,
    pub note: Option<String>,
    pub has_video: bool,
    pub has_audio: bool,
}

/// A ready-made choice for the UI (best, 1080p, audio only, …).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FormatPreset {
    /// yt-dlp format selector.
    pub selector: String,
    pub label: String,
    pub height: Option<u32>,
    pub audio_only: bool,
    #[ts(type = "number | null")]
    pub approx_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VideoInfo {
    /// Present when the URL is (or belongs to) a playlist.
    pub playlist: Option<PlaylistInfo>,
    pub id: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub duration_seconds: Option<f64>,
    pub uploader: Option<String>,
    pub extractor: Option<String>,
    pub webpage_url: String,
    pub is_live: bool,
    pub formats: Vec<VideoFormat>,
    pub presets: Vec<FormatPreset>,
    pub ffmpeg_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaylistEntry {
    pub index: u32,
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub entries: Vec<PlaylistEntry>,
}

/// Allowed audio conversion targets (never passed through unchecked).
pub const AUDIO_FORMATS: [&str; 6] = ["best", "mp3", "m4a", "opus", "flac", "wav"];

/// Max playlist entries listed in the Add dialog.
const MAX_PLAYLIST_ENTRIES: usize = 500;

pub struct VideoEngine {
    pub tools: Arc<ToolLocator>,
}

/// JavaScript runtime for yt-dlp (needed for full YouTube support). yt-dlp
/// only enables Deno by default, so the found runtime is passed explicitly.
/// Returns (runtime name, path).
pub fn find_js_runtime(tools: &ToolLocator, configured: &str) -> Option<(&'static str, PathBuf)> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        if p.is_file() {
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let name = ["deno", "node", "bun"].into_iter().find(|n| stem.starts_with(n)).unwrap_or("deno");
            return Some((name, p));
        }
    }
    ["deno", "node", "bun"].into_iter().find_map(|n| tools.find(n, "").map(|p| (n, p)))
}

fn js_runtime_args(tools: &ToolLocator, configured: &str) -> Vec<String> {
    match find_js_runtime(tools, configured) {
        Some((name, path)) => vec!["--js-runtimes".into(), format!("{name}:{}", path.display())],
        None => vec![],
    }
}

/// Proxy and cookie options shared by inspection and download, so a video
/// that can be inspected can also be downloaded (and the other way round).
pub fn network_args(settings: &Settings) -> Vec<String> {
    let mut a = Vec::new();
    match settings.proxy_mode {
        ProxyMode::None => a.extend(["--proxy".to_string(), String::new()]),
        ProxyMode::Manual if !settings.proxy_url.trim().is_empty() => a.extend(["--proxy".to_string(), settings.proxy_url.trim().to_string()]),
        _ => {}
    }
    let browser = settings.video_cookies_browser.trim();
    if !browser.is_empty() && crate::settings::COOKIE_BROWSERS.contains(&browser) {
        a.extend(["--cookies-from-browser".to_string(), browser.to_string()]);
    }
    a
}

/// Serialises automatic tool setup so two videos never download Deno twice.
static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Makes sure yt-dlp has what it needs for sites like YouTube: a JavaScript
/// runtime (Deno) and, on Windows, FFmpeg for merging the best video and audio
/// streams. Missing pieces are downloaded from their official releases and
/// checksum-verified. Failures are reported but never fail the download:
/// yt-dlp still works for many sites without them.
pub async fn ensure_tools(tools: &ToolLocator, client: &reqwest::Client, settings: &Settings, report: &(dyn Fn(ToolSetup) + Send + Sync)) {
    if !settings.video_auto_setup {
        return;
    }
    let _guard = SETUP_LOCK.lock().await;
    let engines = crate::paths::engines_dir();
    let event = |tool: &str, phase: ToolSetupPhase, downloaded: u64, total: Option<u64>, message: Option<String>| ToolSetup { tool: tool.into(), phase, downloaded, total, message };
    if find_js_runtime(tools, &settings.js_runtime_path).is_none() && tools::deno_asset_name().is_some() {
        report(event("deno", ToolSetupPhase::Downloading, 0, None, None));
        match tools::install_deno(client, &engines).await {
            Ok(_) => report(event("deno", ToolSetupPhase::Installed, 0, None, None)),
            Err(e) => {
                tracing::warn!(error = %e, "automatic Deno setup failed");
                report(event("deno", ToolSetupPhase::Failed, 0, None, Some(e.message.clone())));
            }
        }
    }
    if tools.find("ffmpeg", &settings.ffmpeg_path).is_none() && tools::ffmpeg_asset_name().is_some() {
        report(event("ffmpeg", ToolSetupPhase::Downloading, 0, None, None));
        let progress = |done: u64, total: Option<u64>| report(event("ffmpeg", ToolSetupPhase::Downloading, done, total, None));
        match tools::install_ffmpeg(client, &engines.join("ffmpeg"), &progress).await {
            Ok(_) => report(event("ffmpeg", ToolSetupPhase::Installed, 0, None, None)),
            Err(e) => {
                tracing::warn!(error = %e, "automatic FFmpeg setup failed");
                report(event("ffmpeg", ToolSetupPhase::Failed, 0, None, Some(e.message.clone())));
            }
        }
    }
}

/// The chosen quality first, then fallbacks: formats offered at inspection
/// time can disappear by the time the download starts (YouTube rotates them),
/// and a missing format must not fail the whole download.
pub fn format_selector(chosen: Option<&str>, ffmpeg: bool) -> String {
    let fallback = if ffmpeg { "bv*+ba/b/best*" } else { "b/best*[acodec!=none]/best*" };
    match chosen.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) if c.split('/').any(|alt| alt == fallback.split('/').next().unwrap_or("")) => c.to_string(),
        Some(c) => format!("{c}/{fallback}"),
        None => fallback.to_string(),
    }
}

/// Entries of a flat playlist (`yt-dlp -J --flat-playlist`).
pub fn parse_playlist(json: &serde_json::Value) -> Option<PlaylistInfo> {
    if json.get("_type").and_then(|t| t.as_str()) != Some("playlist") {
        return None;
    }
    let entries = json
        .get("entries")?
        .as_array()?
        .iter()
        .filter(|e| !e.is_null())
        .take(MAX_PLAYLIST_ENTRIES)
        .enumerate()
        .filter_map(|(i, e)| {
            // Flat entries ("_type": "url") carry the page URL in `url`; fully
            // resolved entries (e.g. several <video> tags on one page) carry the
            // media URL in `url` and the page in `webpage_url`.
            let url = opt_str(e, "url").filter(|u| u.starts_with("http")).or_else(|| opt_str(e, "webpage_url"))?;
            Some(PlaylistEntry {
                index: i as u32 + 1,
                id: opt_str(e, "id").unwrap_or_default(),
                title: opt_str(e, "title").unwrap_or_else(|| format!("#{}", i + 1)),
                url,
                duration_seconds: e.get("duration").and_then(|d| d.as_f64()),
            })
        })
        .collect::<Vec<_>>();
    Some(PlaylistInfo {
        id: opt_str(json, "id").unwrap_or_default(),
        title: opt_str(json, "title").unwrap_or_else(|| "Playlist".into()),
        uploader: opt_str(json, "uploader").or_else(|| opt_str(json, "channel")),
        entries,
    })
}

/// Quality choices when the formats of each video are not known yet (playlists).
pub fn generic_presets(ffmpeg: bool) -> Vec<FormatPreset> {
    let mut out = vec![FormatPreset {
        selector: if ffmpeg { "bv*+ba/b".into() } else { "b".into() },
        label: "Best quality".into(),
        height: None,
        audio_only: false,
        approx_size: None,
    }];
    for h in [2160u32, 1440, 1080, 720, 480, 360] {
        let selector = if ffmpeg { format!("bv*[height<={h}]+ba/b[height<={h}]") } else { format!("b[height<={h}]") };
        out.push(FormatPreset { selector, label: format!("{h}p"), height: Some(h), audio_only: false, approx_size: None });
    }
    out.push(FormatPreset { selector: "ba/b".into(), label: "Audio only".into(), height: None, audio_only: true, approx_size: None });
    out
}

fn wants_playlist(u: &url::Url) -> bool {
    u.query_pairs().any(|(k, _)| k == "list") || u.path().contains("/playlist")
}

impl VideoEngine {
    pub fn new(tools: Arc<ToolLocator>) -> Self {
        Self { tools }
    }
}

pub fn partial_dir(save_dir: &Path, id: &str) -> PathBuf {
    save_dir.join(".pixidl-partial").join(security::sanitize_filename(id))
}

fn http_url(url: &str) -> Result<url::Url> {
    let u = security::validate_url(url)?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err(DownloadError::invalid_url("Only http(s) pages can be used with the video engine"));
    }
    Ok(u)
}

fn missing_ytdlp() -> DownloadError {
    DownloadError::new(ErrorKind::EngineUnavailable, "Video engine (yt-dlp) is not installed")
        .with_detail("Install yt-dlp or set its location in Settings → Engines.")
}

/// Maps the last `ERROR:` line of yt-dlp's stderr into a classified error.
pub fn classify_ytdlp_error(stderr: &str) -> DownloadError {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp exited with an error")
        .trim();
    let lower = line.to_ascii_lowercase();
    let all = stderr.to_ascii_lowercase();
    let (kind, msg) = if lower.contains("not a bot") || lower.contains("confirm your age") {
        (ErrorKind::ServerRejected, "YouTube asks to sign in. Choose a browser for cookies in Settings → Engines and stay signed in to YouTube there")
    } else if all.contains("no supported javascript runtime") || all.contains("challenge solving failed") || all.contains("n challenge") || all.contains("signature extraction failed") || all.contains("js runtime") {
        (ErrorKind::EngineUnavailable, "YouTube needs the JavaScript runtime (Deno) and an up-to-date yt-dlp. Check Settings → Engines")
    } else if lower.contains("could not copy") && lower.contains("cookie") || lower.contains("failed to decrypt") && lower.contains("cookie") {
        (ErrorKind::EngineUnavailable, "Could not read the browser's cookies. Close that browser, or choose Firefox, in Settings → Engines")
    } else if lower.contains("unsupported url") {
        (ErrorKind::ExtractorFailed, "This site is not supported by the video extractor")
    } else if lower.contains("http error 404") || lower.contains("not found") || lower.contains("video unavailable") {
        (ErrorKind::NotFound, "The video is unavailable")
    } else if lower.contains("private") || lower.contains("sign in") || lower.contains("login") || lower.contains("members-only") {
        (ErrorKind::ServerRejected, "The video requires an account or is private")
    } else if lower.contains("ffmpeg") {
        (ErrorKind::EngineUnavailable, "FFmpeg is required for this format")
    } else if lower.contains("unable to download webpage") || lower.contains("timed out") || lower.contains("connection") || lower.contains("getaddrinfo") {
        (ErrorKind::NetworkUnavailable, "Network unavailable")
    } else if lower.contains("no space left") {
        (ErrorKind::DiskFull, "Disk full")
    } else if lower.contains("requested format is not available") {
        (ErrorKind::ExtractorFailed, "The selected quality is not available")
    } else {
        (ErrorKind::ExtractorFailed, "Extractor failed")
    };
    // The last lines of stderr (warnings included) help when reporting a problem.
    let tail: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = tail[tail.len().saturating_sub(12)..].join("\n");
    DownloadError::new(kind, msg).with_detail(redact(if tail.is_empty() { line } else { &tail }))
}

fn opt_str(v: &serde_json::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string()).filter(|s| !s.is_empty() && s != "none")
}

fn opt_u64(v: &serde_json::Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)))
}

/// Parses `yt-dlp -J` output.
pub fn parse_info(json: &serde_json::Value, ffmpeg_available: bool) -> Result<VideoInfo> {
    if let Some(pl) = parse_playlist(json) {
        if pl.entries.is_empty() {
            return Err(DownloadError::new(ErrorKind::ExtractorFailed, "The playlist is empty or private"));
        }
        return Ok(VideoInfo {
            id: pl.id.clone(),
            title: pl.title.clone(),
            thumbnail: None,
            duration_seconds: None,
            uploader: pl.uploader.clone(),
            extractor: opt_str(json, "extractor_key").or_else(|| opt_str(json, "extractor")),
            webpage_url: opt_str(json, "webpage_url").unwrap_or_default(),
            is_live: false,
            formats: vec![],
            presets: generic_presets(ffmpeg_available),
            ffmpeg_available,
            playlist: Some(pl),
        });
    }
    let mut formats: Vec<VideoFormat> = json
        .get("formats")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let format_id = opt_str(f, "format_id")?;
                    let vcodec = opt_str(f, "vcodec");
                    let acodec = opt_str(f, "acodec");
                    let protocol = opt_str(f, "protocol").unwrap_or_default();
                    if protocol.contains("mhtml") {
                        return None; // storyboards
                    }
                    let height = opt_u64(f, "height").map(|h| h as u32);
                    let has_video = vcodec.is_some() || (vcodec.is_none() && acodec.is_none() && height.is_some());
                    let has_audio = acodec.is_some() || (vcodec.is_none() && acodec.is_none() && height.is_none());
                    Some(VideoFormat {
                        format_id,
                        ext: opt_str(f, "ext").unwrap_or_else(|| "bin".into()),
                        resolution: opt_str(f, "resolution"),
                        height,
                        fps: f.get("fps").and_then(|x| x.as_f64()),
                        vcodec,
                        acodec,
                        filesize: opt_u64(f, "filesize").or_else(|| opt_u64(f, "filesize_approx")),
                        tbr: f.get("tbr").and_then(|x| x.as_f64()),
                        note: opt_str(f, "format_note"),
                        has_video,
                        has_audio,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if formats.is_empty() {
        // Single-format extractors (e.g. direct media) put the info at top level.
        if let Some(id) = opt_str(json, "format_id").or_else(|| opt_str(json, "ext")) {
            formats.push(VideoFormat {
                format_id: id,
                ext: opt_str(json, "ext").unwrap_or_else(|| "mp4".into()),
                resolution: opt_str(json, "resolution"),
                height: opt_u64(json, "height").map(|h| h as u32),
                fps: None,
                vcodec: opt_str(json, "vcodec"),
                acodec: opt_str(json, "acodec"),
                filesize: opt_u64(json, "filesize").or_else(|| opt_u64(json, "filesize_approx")),
                tbr: None,
                note: None,
                has_video: true,
                has_audio: true,
            });
        }
    }
    let presets = build_presets(&formats, ffmpeg_available);
    Ok(VideoInfo {
        id: opt_str(json, "id").unwrap_or_default(),
        title: opt_str(json, "title").unwrap_or_else(|| "video".into()),
        thumbnail: opt_str(json, "thumbnail").filter(|t| t.starts_with("https://") || t.starts_with("http://")),
        duration_seconds: json.get("duration").and_then(|d| d.as_f64()),
        uploader: opt_str(json, "uploader"),
        extractor: opt_str(json, "extractor_key").or_else(|| opt_str(json, "extractor")),
        webpage_url: opt_str(json, "webpage_url").unwrap_or_default(),
        is_live: json.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false),
        formats,
        presets,
        ffmpeg_available,
        playlist: None,
    })
}

/// Builds the user-facing quality choices. Without FFmpeg, only pre-muxed
/// formats can be offered for video+audio.
pub fn build_presets(formats: &[VideoFormat], ffmpeg: bool) -> Vec<FormatPreset> {
    let mut heights: Vec<u32> = formats
        .iter()
        .filter(|f| f.has_video && (ffmpeg || f.has_audio))
        .filter_map(|f| f.height)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let best_audio_size = formats.iter().filter(|f| f.has_audio && !f.has_video).filter_map(|f| f.filesize).max();
    let size_for = |h: Option<u32>| -> Option<u64> {
        let v = formats
            .iter()
            .filter(|f| f.has_video && h.is_none_or(|h| f.height.is_some_and(|fh| fh <= h)) && (ffmpeg || f.has_audio))
            .max_by_key(|f| (f.height.unwrap_or(0), f.filesize.unwrap_or(0)))?;
        let vs = v.filesize?;
        Some(if v.has_audio { vs } else { vs + best_audio_size.unwrap_or(0) })
    };
    let mut out = Vec::new();
    if ffmpeg {
        out.push(FormatPreset { selector: "bv*+ba/b".into(), label: "Best quality".into(), height: heights.first().copied(), audio_only: false, approx_size: size_for(None) });
    } else {
        out.push(FormatPreset { selector: "b".into(), label: "Best quality".into(), height: heights.first().copied(), audio_only: false, approx_size: size_for(None) });
    }
    for h in heights.iter().take(6) {
        let selector = if ffmpeg { format!("bv*[height<={h}]+ba/b[height<={h}]") } else { format!("b[height<={h}]") };
        out.push(FormatPreset { selector, label: format!("{h}p"), height: Some(*h), audio_only: false, approx_size: size_for(Some(*h)) });
    }
    if formats.iter().any(|f| f.has_audio) {
        out.push(FormatPreset { selector: "ba/b".into(), label: "Audio only".into(), height: None, audio_only: true, approx_size: best_audio_size });
    }
    out
}

/// Parses one `PIXIDLPROG|status|downloaded|total|estimate|speed|eta` line.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressLine {
    pub status: String,
    pub downloaded: Option<u64>,
    pub total: Option<u64>,
    pub speed: Option<u64>,
    pub eta: Option<u64>,
}

pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?;
    let parts: Vec<&str> = rest.split('|').collect();
    if parts.len() < 6 {
        return None;
    }
    let num = |s: &str| -> Option<u64> {
        let s = s.trim();
        if s.is_empty() || s == "NA" || s == "None" {
            return None;
        }
        s.parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64)
    };
    Some(ProgressLine {
        status: parts[0].trim().to_string(),
        downloaded: num(parts[1]),
        total: num(parts[2]).or_else(|| num(parts[3])),
        speed: num(parts[4]),
        eta: num(parts[5]),
    })
}

impl VideoEngine {
    pub fn ytdlp(&self, configured: &str) -> Result<PathBuf> {
        self.tools.find("yt-dlp", configured).ok_or_else(missing_ytdlp)
    }

    /// Metadata for a video, or for a playlist when the URL is one. A video
    /// URL that also names a playlist (`&list=`) gets the playlist attached.
    pub async fn inspect(&self, url: &str, settings: &Settings) -> Result<VideoInfo> {
        let u = http_url(url)?;
        let mut info = self.dump_json(&u, settings, false).await?;
        if info.playlist.is_none() && wants_playlist(&u) {
            if let Ok(pl) = self.dump_json(&u, settings, true).await {
                info.playlist = pl.playlist.filter(|p| p.entries.len() > 1);
            }
        }
        Ok(info)
    }

    async fn dump_json(&self, u: &url::Url, settings: &Settings, flat_playlist: bool) -> Result<VideoInfo> {
        let bin = self.ytdlp(&settings.ytdlp_path)?;
        let ffmpeg = self.tools.find("ffmpeg", &settings.ffmpeg_path);
        let mode: &[&str] = if flat_playlist { &["--yes-playlist", "--flat-playlist"] } else { &["--no-playlist"] };
        let mut cmd = command(&bin);
        cmd.args(["--ignore-config", "--dump-single-json", "--skip-download", "--no-color"])
            .args(mode)
            .args(js_runtime_args(&self.tools, &settings.js_runtime_path))
            .args(network_args(settings))
            .arg("--socket-timeout")
            .arg(settings.read_timeout_secs.to_string());
        if let Some(ff) = &ffmpeg {
            cmd.arg("--ffmpeg-location").arg(ff);
        }
        let child = cmd
            .arg("--")
            .arg(u.as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| missing_ytdlp().with_detail(e.to_string()))?;
        let out = tokio::time::timeout(INSPECT_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| DownloadError::new(ErrorKind::Timeout, "The video extractor timed out"))?
            .map_err(|e| DownloadError::new(ErrorKind::ExtractorFailed, "Extractor failed").with_detail(e.to_string()))?;
        if !out.status.success() {
            return Err(classify_ytdlp_error(&String::from_utf8_lossy(&out.stderr)));
        }
        let json: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| DownloadError::new(ErrorKind::ExtractorFailed, "Extractor returned invalid data").with_detail(e.to_string()))?;
        parse_info(&json, ffmpeg.is_some())
    }

    /// Runs the yt-dlp self-updater (works for the official standalone binaries,
    /// which verify the release checksum themselves).
    pub async fn self_update(&self, ytdlp_path: &str) -> Result<String> {
        let bin = self.ytdlp(ytdlp_path)?;
        let out = tokio::time::timeout(Duration::from_secs(180), command(&bin).args(["--ignore-config", "-U"]).stdout(Stdio::piped()).stderr(Stdio::piped()).output())
            .await
            .map_err(|_| DownloadError::new(ErrorKind::Timeout, "Update timed out"))?
            .map_err(|e| DownloadError::new(ErrorKind::EngineUnavailable, "Could not run yt-dlp").with_detail(e.to_string()))?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(text.trim().to_string())
        } else {
            Err(DownloadError::new(ErrorKind::EngineUnavailable, "yt-dlp could not update itself").with_detail(text.trim().to_string()))
        }
    }
}

impl Engine for VideoEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Video
    }

    fn run(&self, ctx: JobContext) -> BoxFuture<'static, Result<EngineOutcome>> {
        let tools = self.tools.clone();
        Box::pin(run(tools, ctx))
    }

    fn cleanup(&self, d: &Download, delete_completed: bool) -> BoxFuture<'static, ()> {
        let dir = partial_dir(Path::new(&d.save_dir), &d.id);
        let root = Path::new(&d.save_dir).join(".pixidl-partial");
        let full = d.full_path();
        let completed = d.status == crate::types::DownloadStatus::Completed;
        Box::pin(async move {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            let _ = tokio::fs::remove_dir(&root).await; // only if empty
            if delete_completed && completed {
                let _ = tokio::fs::remove_file(&full).await;
            }
        })
    }
}

async fn run(tools: Arc<ToolLocator>, mut ctx: JobContext) -> Result<EngineOutcome> {
    let d = ctx.download.clone();
    let u = http_url(&d.url)?;
    {
        // Logged on the download so the user sees why it is waiting.
        let meta = ctx.meta.clone();
        let report = move |e: ToolSetup| {
            let text = match e.phase {
                ToolSetupPhase::Downloading if e.downloaded == 0 => Some(format!("Setting up {} for video downloads", e.tool)),
                ToolSetupPhase::Installed => Some(format!("Installed {}", e.tool)),
                ToolSetupPhase::Failed => Some(format!("Could not set up {}: {}", e.tool, e.message.unwrap_or_default())),
                _ => None,
            };
            if let Some(t) = text {
                let _ = meta.send(MetaUpdate { event: Some(t), ..Default::default() });
            }
        };
        ensure_tools(&tools, &ctx.http, &ctx.settings, &report).await;
    }
    let bin = tools.find("yt-dlp", &ctx.settings.ytdlp_path).ok_or_else(missing_ytdlp)?;
    let ffmpeg = tools.find("ffmpeg", &ctx.settings.ffmpeg_path);
    let save_dir = PathBuf::from(&d.save_dir);
    let tmp = partial_dir(&save_dir, &d.id);
    tokio::fs::create_dir_all(&tmp).await.map_err(|e| DownloadError::fs("Cannot create destination folder", &e))?;

    let selector = format_selector(d.engine_options.format_id.as_deref(), ffmpeg.is_some());
    let mut cmd = command(&bin);
    cmd.args(["--ignore-config", "--no-playlist", "--no-color", "--newline", "--progress", "--no-simulate", "--continue", "--windows-filenames", "--no-mtime"]);
    cmd.arg("--progress-template").arg(format!(
        "download:{PROGRESS_PREFIX}%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s"
    ));
    cmd.arg("--print").arg(format!("before_dl:{META_PREFIX}%(filesize,filesize_approx|NA)s|%(thumbnail|)s|%(title)s"));
    cmd.arg("--print").arg(format!("after_move:{FILE_PREFIX}%(filepath)s"));
    cmd.arg("-f").arg(&selector);
    cmd.arg("-P").arg(&save_dir);
    cmd.arg("-P").arg(format!("temp:{}", tmp.display()));
    cmd.arg("-o").arg("%(title).150B [%(id)s].%(ext)s");
    cmd.arg("--retries").arg(ctx.settings.retry_count.max(1).to_string());
    cmd.arg("--socket-timeout").arg(ctx.settings.read_timeout_secs.to_string());
    if let Some(ff) = &ffmpeg {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    cmd.args(js_runtime_args(&tools, &ctx.settings.js_runtime_path));
    if d.engine_options.audio_only && ffmpeg.is_some() {
        let fmt = d.engine_options.audio_format.as_deref().filter(|f| AUDIO_FORMATS.contains(f)).unwrap_or("best");
        cmd.args(["-x", "--audio-format", fmt, "--embed-metadata"]);
        if ["mp3", "m4a", "opus", "flac"].contains(&fmt) {
            cmd.arg("--embed-thumbnail");
        }
    } else if d.engine_options.subtitles {
        // English and the UI language; embedded when FFmpeg can remux.
        let lang = if ctx.settings.language == "en" { "en.*".to_string() } else { format!("en.*,{}.*", ctx.settings.language) };
        cmd.args(["--write-subs", "--write-auto-subs", "--sub-langs"]).arg(lang);
        if ffmpeg.is_some() {
            cmd.arg("--embed-subs");
        }
    }
    // yt-dlp enforces its own limit; we pass the stricter of per-download/global.
    let limit = [ctx.download_limiter.rate(), ctx.global_limiter.rate()].into_iter().flatten().min();
    if let Some(l) = limit {
        cmd.arg("--limit-rate").arg(l.to_string());
    }
    cmd.args(network_args(&ctx.settings));
    if let Some(r) = d.referrer.as_deref().filter(|r| r.starts_with("http")) {
        cmd.arg("--referer").arg(r);
    }
    cmd.arg("--").arg(u.as_str());
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| missing_ytdlp().with_detail(e.to_string()))?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if buf.len() < 64 * 1024 {
                buf.push_str(&l);
                buf.push('\n');
            }
        }
        buf
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut finished_streams: u64 = 0;
    let mut meta_total: Option<u64> = None;
    let mut final_path: Option<PathBuf> = None;
    let mut announced = false;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    let mut control = ctx.control.clone();

    loop {
        let line = tokio::select! {
            biased;
            _ = JobContext::stopped(&mut control) => {
                let _ = child.kill().await;
                return Ok(EngineOutcome::Stopped);
            }
            l = lines.next_line() => l,
        };
        let Ok(Some(line)) = line else { break };
        if let Some(rest) = line.strip_prefix(META_PREFIX) {
            let mut parts = rest.splitn(3, '|');
            let size = parts.next().and_then(|s| s.trim().parse::<f64>().ok()).map(|f| f as u64);
            let thumb = parts.next().map(str::trim).filter(|s| s.starts_with("http")).map(String::from);
            let title = parts.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            meta_total = size.or(meta_total);
            ctx.send_meta(MetaUpdate {
                title: title.clone(),
                thumbnail: thumb,
                total_bytes: meta_total,
                resumable: Some(true),
                downloading: true,
                ..Default::default()
            });
            announced = true;
        } else if let Some(p) = parse_progress_line(&line) {
            if !announced {
                ctx.send_meta(MetaUpdate { downloading: true, resumable: Some(true), ..Default::default() });
                announced = true;
            }
            let cur = p.downloaded.unwrap_or(0);
            if p.status == "finished" {
                finished_streams += p.total.unwrap_or(cur).max(cur);
                continue;
            }
            if last_report.elapsed() >= Duration::from_millis(250) {
                last_report = Instant::now();
                let done = finished_streams + cur;
                let total = match (meta_total, p.total) {
                    (Some(m), Some(t)) => Some(m.max(finished_streams + t)),
                    (Some(m), None) => Some(m),
                    (None, Some(t)) => Some(finished_streams + t),
                    (None, None) => None,
                };
                let speed = p.speed.unwrap_or(0);
                ctx.progress.set(EngineProgress {
                    downloaded: done.min(total.unwrap_or(u64::MAX)),
                    total,
                    speed_bps: speed,
                    eta_seconds: p.eta.or_else(|| eta(total, done, speed)),
                    ..Default::default()
                });
            }
        } else if let Some(path) = line.strip_prefix(FILE_PREFIX) {
            final_path = Some(PathBuf::from(path.trim()));
        }
    }

    let status = tokio::select! {
        biased;
        _ = JobContext::stopped(&mut ctx.control) => {
            let _ = child.kill().await;
            return Ok(EngineOutcome::Stopped);
        }
        s = child.wait() => s.map_err(|e| DownloadError::new(ErrorKind::ExtractorFailed, "Extractor failed").with_detail(e.to_string()))?,
    };
    let stderr_text = stderr_task.await.unwrap_or_default();
    if !status.success() {
        return Err(classify_ytdlp_error(&stderr_text));
    }
    let path = final_path.ok_or_else(|| DownloadError::new(ErrorKind::ExtractorFailed, "The extractor did not report the output file").with_detail(redact(&stderr_text)))?;
    // Only accept output that landed inside the destination folder.
    let canon_dir = std::fs::canonicalize(&save_dir).unwrap_or(save_dir.clone());
    let canon_file = std::fs::canonicalize(&path).map_err(|e| DownloadError::fs("Downloaded file not found", &e))?;
    if canon_file.parent() != Some(canon_dir.as_path()) {
        return Err(DownloadError::new(ErrorKind::PermissionDenied, "Extractor wrote outside the destination folder").with_detail(path.display().to_string()));
    }
    let filename = canon_file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let size = tokio::fs::metadata(&canon_file).await.map(|m| m.len()).unwrap_or(0);
    ctx.progress.set(EngineProgress { downloaded: size, total: Some(size), ..Default::default() });
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let _ = tokio::fs::remove_dir(save_dir.join(".pixidl-partial")).await;
    Ok(EngineOutcome::Completed { filename })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        let p = parse_progress_line("PIXIDLPROG|downloading|1024|4096|NA|512.5|6").unwrap();
        assert_eq!(p.downloaded, Some(1024));
        assert_eq!(p.total, Some(4096));
        assert_eq!(p.speed, Some(512));
        assert_eq!(p.eta, Some(6));
        let p = parse_progress_line("PIXIDLPROG|downloading|10|NA|2000.0|NA|NA").unwrap();
        assert_eq!(p.total, Some(2000));
        assert_eq!(p.speed, None);
        assert!(parse_progress_line("[download] 10%").is_none());
        assert!(parse_progress_line("PIXIDLPROG|x").is_none());
    }

    #[test]
    fn error_classification() {
        assert_eq!(classify_ytdlp_error("ERROR: Unsupported URL: https://x").kind, ErrorKind::ExtractorFailed);
        assert_eq!(classify_ytdlp_error("blah\nERROR: [youtube] abc: Video unavailable").kind, ErrorKind::NotFound);
        assert_eq!(classify_ytdlp_error("ERROR: Unable to download webpage: timed out").kind, ErrorKind::NetworkUnavailable);
        let e = classify_ytdlp_error("ERROR: failed https://u:p@h.com/x?sig=secret");
        assert!(!e.detail.unwrap().contains("secret"));
        // YouTube's bot check is not a "private video".
        let e = classify_ytdlp_error("ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser");
        assert!(e.message.contains("cookies"), "{}", e.message);
        // A missing JS runtime shows up as a warning before the actual error.
        let e = classify_ytdlp_error("WARNING: [youtube] No supported JavaScript runtime could be found.\nERROR: [youtube] abc: Requested format is not available");
        assert_eq!(e.kind, ErrorKind::EngineUnavailable);
        assert!(e.detail.unwrap().contains("JavaScript runtime"));
    }

    #[test]
    fn format_selector_has_fallbacks() {
        assert_eq!(format_selector(None, true), "bv*+ba/b/best*");
        assert_eq!(format_selector(None, false), "b/best*[acodec!=none]/best*");
        assert_eq!(format_selector(Some("bv*[height<=720]+ba/b[height<=720]"), true), "bv*[height<=720]+ba/b[height<=720]/bv*+ba/b/best*");
        assert_eq!(format_selector(Some("bv*+ba/b"), true), "bv*+ba/b");
    }

    #[test]
    fn network_args_follow_settings() {
        let mut s = Settings { proxy_mode: ProxyMode::Manual, proxy_url: "http://127.0.0.1:8080".into(), ..Default::default() };
        assert_eq!(network_args(&s), ["--proxy", "http://127.0.0.1:8080"]);
        s.proxy_mode = ProxyMode::None;
        s.video_cookies_browser = "firefox".into();
        assert_eq!(network_args(&s), ["--proxy", "", "--cookies-from-browser", "firefox"]);
        s.proxy_mode = ProxyMode::System;
        s.video_cookies_browser = "--exec=calc".into();
        assert!(network_args(&s).is_empty(), "unknown browsers are never passed to yt-dlp");
    }

    #[test]
    fn info_parsing_and_presets() {
        let j = serde_json::json!({
            "id": "abc", "title": "Clip", "duration": 12.5, "extractor_key": "Generic",
            "webpage_url": "https://example.com/v", "thumbnail": "javascript:alert(1)",
            "formats": [
                {"format_id": "18", "ext": "mp4", "height": 360, "vcodec": "avc1", "acodec": "mp4a", "filesize": 1000},
                {"format_id": "137", "ext": "mp4", "height": 1080, "vcodec": "avc1", "acodec": "none", "filesize": 5000},
                {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "filesize": 300},
                {"format_id": "sb0", "ext": "mhtml", "protocol": "mhtml"}
            ]
        });
        let info = parse_info(&j, true).unwrap();
        assert_eq!(info.formats.len(), 3);
        assert!(info.thumbnail.is_none(), "non-http thumbnails are dropped");
        let labels: Vec<_> = info.presets.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Best quality", "1080p", "360p", "Audio only"]);
        assert_eq!(info.presets[1].approx_size, Some(5300));
        // Without FFmpeg only muxed formats are offered.
        let info = parse_info(&j, false).unwrap();
        let labels: Vec<_> = info.presets.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Best quality", "360p", "Audio only"]);
        assert_eq!(info.presets[0].selector, "b");
        let pl = serde_json::json!({"_type": "playlist", "entries": []});
        assert!(parse_info(&pl, true).is_err());
    }
}
