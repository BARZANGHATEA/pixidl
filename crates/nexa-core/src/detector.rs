//! Best-effort URL classification. The user can always override the engine.
//!
//! ```text
//! magnet:?             → Torrent
//! *.torrent            → Torrent
//! known media site     → Video
//! anything else        → HTTP  (inspection may upgrade an HTML page to Video,
//!                               or an application/x-bittorrent response to Torrent)
//! ```

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::engines::torrent::TorrentInfo;
use crate::engines::video::VideoInfo;
use crate::error::Result;
use crate::security;
use crate::types::{EngineKind, ErrorKind};

/// Hosts whose pages are media pages handled by the extractor. This is a hint
/// only; any page can be tried with the video engine.
const MEDIA_HOSTS: &[&str] = &[
    "youtube.com", "youtu.be", "youtube-nocookie.com", "vimeo.com", "dailymotion.com", "dai.ly", "twitch.tv",
    "soundcloud.com", "bandcamp.com", "tiktok.com", "twitter.com", "x.com", "instagram.com", "facebook.com",
    "fb.watch", "reddit.com", "v.redd.it", "bilibili.com", "nicovideo.jp", "rumble.com", "odysee.com",
    "streamable.com", "vk.com", "ok.ru", "mixcloud.com", "ted.com", "archive.org", "peertube.tv", "aparat.com",
];

/// Extensions that mean "this is a file", even on a media host.
const DIRECT_FILE_EXTS: &[&str] = &[
    "zip", "rar", "7z", "gz", "xz", "bz2", "tar", "iso", "exe", "msi", "dmg", "pkg", "deb", "rpm", "apk", "pdf",
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "csv", "jpg", "jpeg", "png", "gif", "webp", "mp3", "flac",
    "wav", "ogg", "m4a", "mp4", "mkv", "webm", "avi", "mov", "bin", "img",
];

pub fn host_matches(host: &str, list: &[&str]) -> bool {
    let host = host.trim_start_matches("www.").trim_start_matches("m.").to_ascii_lowercase();
    list.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Static classification without any network access.
pub fn detect(input: &str) -> Result<(url::Url, EngineKind)> {
    let url = security::validate_url(input)?;
    if url.scheme() == "magnet" {
        return Ok((url, EngineKind::Torrent));
    }
    let last = url.path_segments().and_then(|s| s.filter(|x| !x.is_empty()).last()).unwrap_or("").to_ascii_lowercase();
    let ext = security::extension_of(&last);
    if ext == "torrent" {
        return Ok((url, EngineKind::Torrent));
    }
    if url.scheme() != "ftp" {
        if let Some(host) = url.host_str() {
            if host_matches(host, MEDIA_HOSTS) && !DIRECT_FILE_EXTS.contains(&ext.as_str()) {
                return Ok((url, EngineKind::Video));
            }
        }
    }
    Ok((url, EngineKind::Http))
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InspectionError {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: Option<String>,
}

impl From<crate::error::DownloadError> for InspectionError {
    fn from(e: crate::error::DownloadError) -> Self {
        Self { kind: e.kind, message: e.message, detail: e.detail }
    }
}

/// Everything the Add dialog needs to know about a URL.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UrlInspection {
    pub url: String,
    pub final_url: Option<String>,
    pub engine: EngineKind,
    /// Other engines that can reasonably be tried.
    pub alternatives: Vec<EngineKind>,
    pub filename: Option<String>,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    pub content_type: Option<String>,
    pub resumable: Option<bool>,
    pub category: String,
    pub video: Option<VideoInfo>,
    pub torrent: Option<TorrentInfo>,
    /// A non-fatal problem (e.g. extractor missing, metadata timeout).
    pub warning: Option<InspectionError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_urls() {
        let k = |u: &str| detect(u).unwrap().1;
        assert_eq!(k("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x"), EngineKind::Torrent);
        assert_eq!(k("https://example.com/linux.iso.torrent"), EngineKind::Torrent);
        assert_eq!(k("https://www.youtube.com/watch?v=abc"), EngineKind::Video);
        assert_eq!(k("https://youtu.be/abc"), EngineKind::Video);
        assert_eq!(k("https://m.youtube.com/watch?v=abc"), EngineKind::Video);
        assert_eq!(k("https://player.vimeo.com/video/1"), EngineKind::Video);
        assert_eq!(k("https://archive.org/download/x/file.zip"), EngineKind::Http);
        assert_eq!(k("https://example.com/file.zip"), EngineKind::Http);
        assert_eq!(k("https://notyoutube.com/watch"), EngineKind::Http);
        assert_eq!(k("ftp://example.com/x.bin"), EngineKind::Http);
        assert!(detect("javascript:alert(1)").is_err());
    }
}
