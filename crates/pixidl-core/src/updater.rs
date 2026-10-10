//! App updates from the project's GitHub releases.
//!
//! - [`Updater::check`] lists the releases through the GitHub REST API, skips
//!   drafts (and pre-releases unless asked for) and picks the highest semantic
//!   version; the Windows installer (`*_x64-setup.exe`) and its SHA-256 from the
//!   release's `SHA256SUMS.txt` are attached to the result
//! - [`Updater::download`] streams the installer to `<dir>/<name>.part`, hashes
//!   it on the fly and renames it into place only when the hash matches the
//!   published one. A release without a published checksum cannot be
//!   downloaded for installation at all
//! - every URL, and every redirect hop, must be on GitHub's hosts
//!   ([`HostPolicy::github`]); the tests point the updater at a local server

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::header::{HeaderMap, ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
pub use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::engines::SpeedMeter;
use crate::error::{DownloadError, Result};
use crate::settings::Settings;
use crate::types::ErrorKind;

pub const REPO_URL: &str = "https://github.com/BARZANGHATEA/pixidl";
pub const RELEASES_URL: &str = "https://github.com/BARZANGHATEA/pixidl/releases";
pub const API_REPO_URL: &str = "https://api.github.com/repos/BARZANGHATEA/pixidl";
/// Suffix of the NSIS installer the release workflow publishes.
pub const INSTALLER_SUFFIX: &str = "_x64-setup.exe";
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";
pub const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_API_BODY: usize = 8 * 1024 * 1024;
const MAX_SUMS_BODY: usize = 64 * 1024;
const MAX_NOTES_CHARS: usize = 20_000;
const API_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATER_AGENT: &str = concat!("pixidl/", env!("CARGO_PKG_VERSION"), " (updater; +https://github.com/BARZANGHATEA/pixidl)");

// ---------------------------------------------------------------- types

/// A downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateAsset {
    pub name: String,
    pub url: String,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReleaseInfo {
    /// Semantic version without the leading `v`.
    pub version: String,
    pub tag: String,
    pub name: String,
    /// Release notes (Markdown source; the UI renders it as text).
    pub notes: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub prerelease: bool,
    /// The Windows installer, if the release has one.
    pub installer: Option<UpdateAsset>,
    /// Lower-case hex SHA-256 of the installer from the release's `SHA256SUMS.txt`.
    pub sha256: Option<String>,
}

/// Why an available update can't be downloaded and installed from inside the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InstallBlocker {
    /// Only the Windows installer is published; other platforms get the release page.
    UnsupportedPlatform,
    NoInstaller,
    /// No `SHA256SUMS.txt` entry for the installer: it can't be verified.
    NoChecksum,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    pub current: String,
    /// Newest eligible release; `None` when nothing has been published.
    pub latest: Option<ReleaseInfo>,
    pub update_available: bool,
    pub install_blocker: Option<InstallBlocker>,
}

/// Updater events, forwarded to the UI on their own channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateEvent {
    /// A background check found a newer version.
    Available { info: Box<UpdateInfo> },
    Progress {
        #[ts(type = "number")]
        downloaded: u64,
        #[ts(type = "number | null")]
        total: Option<u64>,
        #[serde(rename = "speedBps")]
        #[ts(type = "number")]
        speed_bps: u64,
    },
    /// The download finished; its checksum is being checked.
    Verifying,
    /// The verified installer is ready to run.
    Ready { version: String },
}

// ---------------------------------------------------------------- pure helpers

/// Parses `v1.2.3`, `1.2.3-beta.1`, … into a semantic version.
pub fn parse_version(tag: &str) -> Option<semver::Version> {
    let t = tag.trim();
    let t = t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t);
    semver::Version::parse(t).ok()
}

/// Version precedence (build metadata ignored).
pub fn compare_versions(a: &semver::Version, b: &semver::Version) -> Ordering {
    a.cmp_precedence(b)
}

/// `true` when `latest` is newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => compare_versions(&l, &c) == Ordering::Greater,
        _ => false,
    }
}

/// A bare file name we are willing to write into the updates folder.
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

/// Highest eligible release: drafts and untagged releases are skipped, and so
/// are pre-releases (flagged by GitHub or by a `-pre` version) unless asked for.
pub fn select_release(releases: &[GhRelease], include_prereleases: bool) -> Option<(semver::Version, &GhRelease)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| parse_version(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, r)| include_prereleases || (!r.prerelease && v.pre.is_empty()))
        .max_by(|(a, _), (b, _)| compare_versions(a, b))
}

/// The Windows x64 installer among a release's assets.
pub fn pick_installer(assets: &[GhAsset]) -> Option<&GhAsset> {
    let candidates: Vec<&GhAsset> = assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(INSTALLER_SUFFIX) && is_safe_file_name(&a.name))
        .collect();
    candidates.iter().find(|a| a.name.starts_with("pixidl_")).or(candidates.first()).copied()
}

/// Finds `asset`'s hash in a `SHA256SUMS.txt` (`<sha256>  <file>` per line).
pub fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    crate::tools::checksum_for(sums, asset)
}

fn blocker_for(r: &ReleaseInfo, platform_supported: bool) -> Option<InstallBlocker> {
    if !platform_supported {
        Some(InstallBlocker::UnsupportedPlatform)
    } else if r.installer.is_none() {
        Some(InstallBlocker::NoInstaller)
    } else if r.sha256.is_none() {
        Some(InstallBlocker::NoChecksum)
    } else {
        None
    }
}

/// The installer is a Windows NSIS setup; elsewhere the user gets the release page.
pub const PLATFORM_SUPPORTED: bool = cfg!(windows);

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// GitHub answers 403 (or 429) with `x-ratelimit-remaining: 0` when the
/// unauthenticated limit (60 requests/hour per IP) is used up.
pub fn rate_limit_error(status: u16, h: &HeaderMap) -> Option<DownloadError> {
    let remaining = h.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).map(str::trim);
    let limited = match status {
        429 => true,
        403 => remaining == Some("0") || h.contains_key("retry-after"),
        _ => false,
    };
    if !limited {
        return None;
    }
    let reset = h
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0));
    let detail = match reset {
        Some(t) => format!("HTTP status {status}; the limit resets at {}", t.to_rfc3339()),
        None => format!("HTTP status {status}"),
    };
    Some(DownloadError::new(ErrorKind::ServerRejected, "GitHub's request limit was reached. Try again in a while.").with_detail(detail))
}

// ---------------------------------------------------------------- host policy

/// Which hosts the updater may talk to.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    /// Hosts a request may be sent to directly (API and asset URLs).
    pub hosts: Vec<String>,
    /// Extra hosts a redirect may lead to; an entry starting with `.` matches subdomains.
    pub redirect_hosts: Vec<String>,
    pub https_only: bool,
}

impl HostPolicy {
    /// api.github.com / github.com, and GitHub's asset CDN
    /// (objects.githubusercontent.com, release-assets.githubusercontent.com) for redirects.
    pub fn github() -> Self {
        Self {
            hosts: vec!["api.github.com".into(), "github.com".into()],
            redirect_hosts: vec![".githubusercontent.com".into()],
            https_only: true,
        }
    }

    fn scheme_ok(&self, u: &url::Url) -> bool {
        (u.scheme() == "https" || (!self.https_only && u.scheme() == "http")) && u.username().is_empty() && u.password().is_none()
    }

    fn host(u: &url::Url) -> Option<String> {
        u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
    }

    /// May a request start at `u`?
    pub fn allows(&self, u: &url::Url) -> bool {
        self.scheme_ok(u) && Self::host(u).is_some_and(|h| self.hosts.iter().any(|p| host_matches(&h, p)))
    }

    /// May a redirect lead to `u`?
    pub fn allows_redirect(&self, u: &url::Url) -> bool {
        self.allows(u) || (self.scheme_ok(u) && Self::host(u).is_some_and(|h| self.redirect_hosts.iter().any(|p| host_matches(&h, p))))
    }
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    if pattern.starts_with('.') {
        host.len() > pattern.len() && host.ends_with(&pattern)
    } else {
        host == pattern
    }
}

fn untrusted(u: &str) -> DownloadError {
    DownloadError::new(ErrorKind::PermissionDenied, "The update address is not on GitHub; refusing to use it").with_detail(crate::error::redact_url(u))
}

// ---------------------------------------------------------------- updater

/// Talks to the GitHub releases API and downloads verified installers.
#[derive(Clone)]
pub struct Updater {
    client: reqwest::Client,
    policy: HostPolicy,
    api_repo: String,
}

impl Updater {
    /// The real project on GitHub, through the user's proxy settings.
    pub fn github(settings: &Settings) -> Result<Self> {
        Self::new(settings, HostPolicy::github(), API_REPO_URL)
    }

    /// `api_repo` is the API URL of the repository (`…/repos/<owner>/<name>`).
    pub fn new(settings: &Settings, policy: HostPolicy, api_repo: impl Into<String>) -> Result<Self> {
        let api_repo = api_repo.into().trim_end_matches('/').to_string();
        match url::Url::parse(&api_repo) {
            Ok(u) if policy.allows(&u) => {}
            _ => return Err(untrusted(&api_repo)),
        }
        let p = policy.clone();
        let client = crate::engines::http::client_builder(settings)?
            .read_timeout(Duration::from_secs(settings.read_timeout_secs as u64))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects")
                } else if p.allows_redirect(attempt.url()) {
                    attempt.follow()
                } else {
                    let host = attempt.url().host_str().unwrap_or("").to_string();
                    attempt.error(format!("redirect to an untrusted host ({host})"))
                }
            }))
            .build()
            .map_err(|e| DownloadError::new(ErrorKind::Unknown, "Cannot create HTTP client").with_detail(e.to_string()))?;
        Ok(Self { client, policy, api_repo })
    }

    async fn get(&self, url: &str, accept: &str, api: bool) -> Result<reqwest::Response> {
        let u = url::Url::parse(url).map_err(|_| untrusted(url))?;
        if !self.policy.allows(&u) {
            return Err(untrusted(url));
        }
        let mut rb = self.client.get(u).header(USER_AGENT, UPDATER_AGENT).header(ACCEPT, accept);
        if api {
            rb = rb.header("X-GitHub-Api-Version", "2022-11-28").timeout(API_TIMEOUT);
        }
        let r = rb.send().await.map_err(|e| request_error(&e))?;
        if !self.policy.allows_redirect(r.url()) {
            return Err(untrusted(r.url().as_str()));
        }
        Ok(r)
    }

    /// Looks for a newer release than `current`.
    pub async fn check(&self, current: &str, include_prereleases: bool) -> Result<UpdateInfo> {
        let cur = parse_version(current).ok_or_else(|| DownloadError::new(ErrorKind::Unknown, "Invalid application version").with_detail(current.to_string()))?;
        let none = UpdateInfo { current: current.to_string(), latest: None, update_available: false, install_blocker: None };
        let r = self.get(&format!("{}/releases?per_page=30", self.api_repo), "application/vnd.github+json", true).await?;
        let status = r.status().as_u16();
        if status == 404 {
            // Unknown repository or nothing published yet.
            return Ok(none);
        }
        if let Some(e) = rate_limit_error(status, r.headers()) {
            return Err(e);
        }
        if !r.status().is_success() {
            return Err(DownloadError::new(ErrorKind::ServerRejected, "GitHub could not list the releases").with_detail(format!("HTTP status {status}")));
        }
        let body = read_limited(r, MAX_API_BODY).await?;
        let releases: Vec<GhRelease> = serde_json::from_slice(&body)
            .map_err(|e| DownloadError::new(ErrorKind::ServerRejected, "Unexpected response from GitHub").with_detail(e.to_string()))?;
        let Some((version, rel)) = select_release(&releases, include_prereleases) else { return Ok(none) };

        let installer = pick_installer(&rel.assets)
            .filter(|a| url::Url::parse(&a.browser_download_url).is_ok_and(|u| self.policy.allows(&u)))
            .map(|a| UpdateAsset { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size });
        let mut sha256 = None;
        if let (Some(inst), Some(sums)) = (&installer, rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case(CHECKSUMS_ASSET))) {
            let r = self.get(&sums.browser_download_url, "application/octet-stream", false).await?;
            let status = r.status().as_u16();
            if let Some(e) = rate_limit_error(status, r.headers()) {
                return Err(e);
            }
            if !r.status().is_success() {
                return Err(DownloadError::new(ErrorKind::ServerRejected, "Could not download the release checksums").with_detail(format!("HTTP status {status}")));
            }
            let text = read_limited(r, MAX_SUMS_BODY).await?;
            sha256 = checksum_for(&String::from_utf8_lossy(&text), &inst.name);
        }
        let name = rel.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&rel.tag_name).to_string();
        let html_url = if rel.html_url.starts_with(REPO_URL) || !self.policy.https_only { rel.html_url.clone() } else { RELEASES_URL.to_string() };
        let latest = ReleaseInfo {
            version: version.to_string(),
            tag: rel.tag_name.clone(),
            name,
            notes: truncate_chars(rel.body.as_deref().unwrap_or("").trim(), MAX_NOTES_CHARS),
            published_at: rel.published_at.clone(),
            html_url,
            prerelease: rel.prerelease || !version.pre.is_empty(),
            installer,
            sha256,
        };
        let update_available = compare_versions(&version, &cur) == Ordering::Greater;
        let install_blocker = blocker_for(&latest, PLATFORM_SUPPORTED);
        Ok(UpdateInfo { current: current.to_string(), latest: Some(latest), update_available, install_blocker })
    }

    /// Downloads `asset` into `dest_dir` and verifies it against `sha256`.
    /// Returns the path of the verified file. Without a checksum nothing is
    /// downloaded; a mismatch deletes the file.
    pub async fn download(
        &self,
        asset: &UpdateAsset,
        sha256: Option<&str>,
        dest_dir: &Path,
        progress: &(dyn Fn(UpdateEvent) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        let expected = sha256
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| DownloadError::new(ErrorKind::ChecksumMismatch, "This release has no published checksum, so it can't be installed automatically"))?;
        if !is_safe_file_name(&asset.name) {
            return Err(DownloadError::new(ErrorKind::InvalidUrl, "Invalid installer name").with_detail(asset.name.clone()));
        }
        if asset.size > MAX_INSTALLER_BYTES {
            return Err(DownloadError::new(ErrorKind::ServerRejected, "The installer is unexpectedly large"));
        }
        let fs_err = |m: &'static str| move |e: std::io::Error| DownloadError::fs(m, &e);
        tokio::fs::create_dir_all(dest_dir).await.map_err(fs_err("Cannot create the updates folder"))?;
        let target = dest_dir.join(&asset.name);
        let part = dest_dir.join(format!("{}.part", asset.name));
        clear_dir_except(dest_dir, &asset.name);
        if target.is_file() {
            if verify_file(&target, &expected).is_ok() {
                return Ok(target);
            }
            let _ = std::fs::remove_file(&target);
        }
        if cancel.is_cancelled() {
            return Err(DownloadError::cancelled());
        }

        let r = tokio::select! {
            r = self.get(&asset.url, "application/octet-stream", false) => r?,
            _ = cancel.cancelled() => return Err(DownloadError::cancelled()),
        };
        let status = r.status().as_u16();
        if let Some(e) = rate_limit_error(status, r.headers()) {
            return Err(e);
        }
        if !r.status().is_success() {
            return Err(DownloadError::from_status(status).with_detail(format!("HTTP status {status} for {}", asset.name)));
        }
        let total = r.content_length().or((asset.size > 0).then_some(asset.size));
        if total.is_some_and(|t| t > MAX_INSTALLER_BYTES) {
            return Err(DownloadError::new(ErrorKind::ServerRejected, "The installer is unexpectedly large"));
        }

        let result = async {
            let mut file = tokio::fs::File::create(&part).await.map_err(fs_err("Cannot write the update"))?;
            let mut hasher = Sha256::new();
            let mut downloaded: u64 = 0;
            let mut meter = SpeedMeter::new(Duration::from_secs(3));
            let mut last_emit = Instant::now();
            let mut stream = r.bytes_stream();
            progress(UpdateEvent::Progress { downloaded: 0, total, speed_bps: 0 });
            loop {
                let chunk = tokio::select! {
                    c = stream.next() => c,
                    _ = cancel.cancelled() => return Err(DownloadError::cancelled()),
                };
                let Some(chunk) = chunk else { break };
                let chunk = chunk.map_err(|e| request_error(&e))?;
                downloaded += chunk.len() as u64;
                if downloaded > MAX_INSTALLER_BYTES || total.is_some_and(|t| downloaded > t) {
                    return Err(DownloadError::new(ErrorKind::ServerRejected, "The server sent more data than announced"));
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await.map_err(fs_err("Cannot write the update"))?;
                let speed = meter.record(downloaded);
                if last_emit.elapsed() >= Duration::from_millis(200) {
                    last_emit = Instant::now();
                    progress(UpdateEvent::Progress { downloaded, total, speed_bps: speed });
                }
            }
            file.flush().await.map_err(fs_err("Cannot write the update"))?;
            file.sync_all().await.map_err(fs_err("Cannot write the update"))?;
            drop(file);
            progress(UpdateEvent::Progress { downloaded, total, speed_bps: 0 });
            if total.is_some_and(|t| downloaded != t) {
                return Err(DownloadError::new(ErrorKind::NetworkUnavailable, "The download was incomplete").with_detail(format!("{downloaded} of {} bytes", total.unwrap_or(0))));
            }
            progress(UpdateEvent::Verifying);
            let actual = hex::encode(hasher.finalize());
            if actual != expected {
                return Err(DownloadError::new(ErrorKind::ChecksumMismatch, "The downloaded update failed checksum verification and was deleted")
                    .with_detail(format!("expected {expected}, got {actual}")));
            }
            let _ = tokio::fs::remove_file(&target).await;
            tokio::fs::rename(&part, &target).await.map_err(fs_err("Cannot save the update"))?;
            Ok(target.clone())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result
    }
}

fn request_error(e: &reqwest::Error) -> DownloadError {
    if e.is_redirect() {
        return DownloadError::new(ErrorKind::PermissionDenied, "The download was redirected away from GitHub; refusing to follow it").with_detail(crate::error::redact(&format!("{e:#}")));
    }
    DownloadError::from_reqwest(e)
}

async fn read_limited(r: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut stream = r.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| request_error(&e))?;
        if out.len() + chunk.len() > max {
            return Err(DownloadError::new(ErrorKind::ServerRejected, "Unexpectedly large response from GitHub"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Re-hashes a downloaded file (done again right before it is executed).
pub fn verify_file(path: &Path, sha256: &str) -> Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| DownloadError::fs("Cannot read the update", &e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| DownloadError::fs("Cannot read the update", &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(sha256.trim()) {
        Ok(())
    } else {
        Err(DownloadError::new(ErrorKind::ChecksumMismatch, "The update file changed after it was verified").with_detail(format!("expected {sha256}, got {actual}")))
    }
}

/// Removes everything in the updates folder except `keep` (old installers,
/// interrupted downloads). The folder only ever holds updater files.
fn clear_dir_except(dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name != keep && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Deletes downloaded installers (after an update was installed or abandoned).
pub fn clear_downloads(dir: &Path) {
    clear_dir_except(dir, "");
}

/// `<data_dir>/updates`.
pub fn updates_dir() -> PathBuf {
    crate::paths::data_dir().join("updates")
}

/// Key of the last automatic/manual check time in the db key-value store.
pub const LAST_CHECK_KEY: &str = "updates.last_check";
/// How often the automatic check runs.
pub const CHECK_INTERVAL_HOURS: i64 = 24;

/// Whether an automatic check is due, given the stored RFC 3339 time of the
/// last one (missing or unreadable → due; a time in the future → due too, so a
/// wrong clock can't disable checks for good).
pub fn check_due(last_check: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(last) = last_check.and_then(|s| chrono::DateTime::parse_from_rfc3339(s.trim()).ok()) else { return true };
    let elapsed = now.signed_duration_since(last.with_timezone(&chrono::Utc));
    elapsed < chrono::Duration::zero() || elapsed >= chrono::Duration::hours(CHECK_INTERVAL_HOURS)
}

/// [`check_due`] at the current time.
pub fn check_due_now(last_check: Option<&str>) -> bool {
    check_due(last_check, chrono::Utc::now())
}

/// Timestamp format stored under [`LAST_CHECK_KEY`].
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, draft: bool, pre: bool, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            name: Some(format!("pixidl {tag}")),
            body: Some("notes".into()),
            draft,
            prerelease: pre,
            published_at: Some("2026-10-01T00:00:00Z".into()),
            html_url: format!("{RELEASES_URL}/tag/{tag}"),
            assets: assets.iter().map(|n| GhAsset { name: (*n).into(), browser_download_url: format!("{RELEASES_URL}/download/{tag}/{n}"), size: 10 }).collect(),
        }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v1.2.3").unwrap(), semver::Version::new(1, 2, 3));
        assert_eq!(parse_version(" V1.2.3 ").unwrap(), semver::Version::new(1, 2, 3));
        assert_eq!(parse_version("1.2.3").unwrap(), semver::Version::new(1, 2, 3));
        assert_eq!(parse_version("v1.2.0-beta.2").unwrap().pre.as_str(), "beta.2");
        assert!(parse_version("v1.2").is_none());
        assert!(parse_version("latest").is_none());
        assert!(parse_version("").is_none());
    }

    #[test]
    fn orders_versions() {
        let v = |s: &str| parse_version(s).unwrap();
        assert_eq!(compare_versions(&v("1.10.0"), &v("1.9.9")), Ordering::Greater);
        assert_eq!(compare_versions(&v("1.2.0"), &v("1.2.0-rc.1")), Ordering::Greater);
        assert_eq!(compare_versions(&v("1.2.0-rc.1"), &v("1.2.0-beta.10")), Ordering::Greater);
        assert_eq!(compare_versions(&v("1.2.0-beta.10"), &v("1.2.0-beta.2")), Ordering::Greater);
        assert_eq!(compare_versions(&v("1.2.0+build.5"), &v("1.2.0")), Ordering::Equal);
        assert!(is_newer("v1.1.1", "1.1.0"));
        assert!(!is_newer("v1.1.0", "1.1.0"));
        assert!(!is_newer("v1.0.9", "1.1.0"));
        assert!(!is_newer("garbage", "1.1.0"));
    }

    #[test]
    fn selects_highest_eligible_release() {
        let list = vec![
            rel("v1.1.0", false, false, &[]),
            rel("v9.0.0", true, false, &[]),       // draft
            rel("v1.3.0-beta.1", false, false, &[]), // pre-release by version only
            rel("v1.2.5", false, true, &[]),       // pre-release by flag
            rel("v1.2.0", false, false, &[]),
            rel("nightly", false, false, &[]),     // not a version
        ];
        assert_eq!(select_release(&list, false).unwrap().1.tag_name, "v1.2.0");
        assert_eq!(select_release(&list, true).unwrap().1.tag_name, "v1.3.0-beta.1");
        assert!(select_release(&[], false).is_none());
        assert!(select_release(&[rel("v2.0.0", true, false, &[])], true).is_none());
    }

    #[test]
    fn picks_the_windows_installer() {
        let r = rel("v1.2.0", false, false, &["pixidl-chromium.zip", "pixidl-firefox.xpi", "pixidl_1.2.0_x64-setup.exe", "SHA256SUMS.txt"]);
        assert_eq!(pick_installer(&r.assets).unwrap().name, "pixidl_1.2.0_x64-setup.exe");
        let r = rel("v1.2.0", false, false, &["other_1.2.0_x64-setup.exe", "pixidl_1.2.0_x64-setup.exe"]);
        assert_eq!(pick_installer(&r.assets).unwrap().name, "pixidl_1.2.0_x64-setup.exe");
        let r = rel("v1.2.0", false, false, &["pixidl_1.2.0_x86-setup.exe", "pixidl_1.2.0_x64_en-US.msi", "../evil_x64-setup.exe"]);
        assert!(pick_installer(&r.assets).is_none());
    }

    #[test]
    fn parses_sha256sums() {
        // As written by the release workflow (PowerShell, CRLF).
        let h1 = "a".repeat(64);
        let h2 = "B".repeat(64);
        let sums = format!("{h1}  pixidl-chromium.zip\r\n{h2}  pixidl_1.2.0_x64-setup.exe\r\nnot-a-hash  pixidl-firefox.xpi\r\n");
        assert_eq!(checksum_for(&sums, "pixidl_1.2.0_x64-setup.exe").unwrap(), "b".repeat(64));
        assert_eq!(checksum_for(&sums, "pixidl-chromium.zip").unwrap(), h1);
        assert!(checksum_for(&sums, "pixidl-firefox.xpi").is_none());
        assert!(checksum_for(&sums, "missing.exe").is_none());
        assert_eq!(checksum_for(&format!("{h1} *setup_x64-setup.exe\n"), "setup_x64-setup.exe").unwrap(), h1);
    }

    #[test]
    fn host_policy() {
        let p = HostPolicy::github();
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(p.allows(&u("https://api.github.com/repos/a/b/releases")));
        assert!(p.allows(&u("https://github.com/BARZANGHATEA/pixidl/releases/download/v1/x.exe")));
        assert!(!p.allows(&u("http://github.com/x")));
        assert!(!p.allows(&u("https://github.com.evil.example/x")));
        assert!(!p.allows(&u("https://user:pw@github.com/x")));
        assert!(!p.allows(&u("https://objects.githubusercontent.com/x")), "CDN only via redirect");
        assert!(p.allows_redirect(&u("https://objects.githubusercontent.com/x")));
        assert!(p.allows_redirect(&u("https://release-assets.githubusercontent.com/x")));
        assert!(!p.allows_redirect(&u("https://githubusercontent.com.evil.example/x")));
        assert!(!p.allows_redirect(&u("https://evilgithubusercontent.com/x")));
        assert!(!p.allows_redirect(&u("http://objects.githubusercontent.com/x")));
        assert!(!p.allows_redirect(&u("https://example.com/x")));
    }

    #[test]
    fn safe_file_names() {
        assert!(is_safe_file_name("pixidl_1.2.0_x64-setup.exe"));
        assert!(!is_safe_file_name("../x.exe"));
        assert!(!is_safe_file_name("a\\b.exe"));
        assert!(!is_safe_file_name(".hidden"));
        assert!(!is_safe_file_name(""));
        assert!(!is_safe_file_name("a b.exe"));
    }

    #[test]
    fn detects_rate_limiting() {
        let mut h = HeaderMap::new();
        assert!(rate_limit_error(403, &h).is_none(), "plain 403 is not a rate limit");
        h.insert("x-ratelimit-remaining", "0".parse().unwrap());
        h.insert("x-ratelimit-reset", "1790000000".parse().unwrap());
        let e = rate_limit_error(403, &h).unwrap();
        assert!(e.message.contains("limit"));
        assert!(e.detail.unwrap().contains("resets at"));
        assert!(rate_limit_error(429, &HeaderMap::new()).is_some());
        assert!(rate_limit_error(200, &h).is_none());
    }

    #[test]
    fn install_blockers() {
        let mut r = ReleaseInfo {
            version: "1.2.0".into(),
            tag: "v1.2.0".into(),
            name: "x".into(),
            notes: String::new(),
            published_at: None,
            html_url: RELEASES_URL.into(),
            prerelease: false,
            installer: Some(UpdateAsset { name: "pixidl_1.2.0_x64-setup.exe".into(), url: "https://github.com/x".into(), size: 1 }),
            sha256: Some("a".repeat(64)),
        };
        assert_eq!(blocker_for(&r, true), None);
        assert_eq!(blocker_for(&r, false), Some(InstallBlocker::UnsupportedPlatform));
        r.sha256 = None;
        assert_eq!(blocker_for(&r, true), Some(InstallBlocker::NoChecksum));
        r.installer = None;
        assert_eq!(blocker_for(&r, true), Some(InstallBlocker::NoInstaller));
    }

    #[test]
    fn automatic_check_interval() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        assert!(check_due(None, now));
        assert!(check_due(Some("garbage"), now));
        assert!(!check_due(Some("2026-10-10T01:00:00Z"), now));
        assert!(!check_due(Some("2026-10-10T14:00:00+03:30"), now), "10:30 UTC, 1.5 h ago");
        assert!(check_due(Some("2026-10-09T12:00:00Z"), now));
        assert!(check_due(Some("2026-10-11T12:00:00Z"), now), "future time (clock changed)");
    }

    #[test]
    fn truncates_notes_on_char_boundaries() {
        assert_eq!(truncate_chars("سلام دنیا", 4), "سلام…");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }
}
