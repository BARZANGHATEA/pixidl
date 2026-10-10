//! Domain types shared by the core, the Tauri shell and (via ts-rs) the frontend.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Lifecycle state of a download.
///
/// Transitions are validated by [`DownloadStatus::can_transition_to`]; the manager
/// refuses any transition that is not listed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DownloadStatus {
    /// Waiting in the queue for a free slot (or for its scheduled time).
    Queued,
    /// Connecting / fetching metadata.
    Preparing,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl DownloadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "preparing" => Self::Preparing,
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// True while an engine task owns the download.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Preparing | Self::Downloading)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// The explicit state machine.
    ///
    /// ```text
    /// Queued ──► Preparing ──► Downloading ──► Completed
    ///   │  ▲         │  │          │  │  │
    ///   │  │         │  │          │  │  └──► Failed ──► Queued (retry)
    ///   │  └─────────┴──┴──────────┘  └─────► Cancelled ──► Queued (restart)
    ///   └──► Paused ◄──────────────────────── (pause from any active state)
    /// ```
    pub fn can_transition_to(self, next: Self) -> bool {
        use DownloadStatus::*;
        matches!(
            (self, next),
            (Queued, Preparing)
                | (Queued, Paused)
                | (Queued, Cancelled)
                | (Queued, Failed)
                | (Preparing, Downloading)
                | (Preparing, Paused)
                | (Preparing, Failed)
                | (Preparing, Cancelled)
                | (Preparing, Queued)
                | (Preparing, Completed)
                | (Downloading, Paused)
                | (Downloading, Failed)
                | (Downloading, Cancelled)
                | (Downloading, Completed)
                | (Downloading, Queued)
                | (Paused, Queued)
                | (Paused, Cancelled)
                | (Failed, Queued)
                | (Failed, Cancelled)
                | (Cancelled, Queued)
                | (Completed, Queued)
        )
    }
}

impl std::fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which engine performs the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EngineKind {
    Http,
    Torrent,
    Video,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Torrent => "torrent",
            Self::Video => "video",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "http" => Self::Http,
            "torrent" => Self::Torrent,
            "video" => Self::Video,
            _ => return None,
        })
    }
}

/// Machine-readable error classification. The UI translates these into
/// human-readable messages; the technical detail is kept separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorKind {
    NetworkUnavailable,
    Timeout,
    ServerRejected,
    NotFound,
    PermissionDenied,
    DiskFull,
    ResumeNotSupported,
    ExtractorFailed,
    ChecksumMismatch,
    EngineUnavailable,
    TorrentMetadataUnavailable,
    InvalidUrl,
    Filesystem,
    Cancelled,
    Unknown,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NetworkUnavailable => "network_unavailable",
            Self::Timeout => "timeout",
            Self::ServerRejected => "server_rejected",
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::DiskFull => "disk_full",
            Self::ResumeNotSupported => "resume_not_supported",
            Self::ExtractorFailed => "extractor_failed",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::EngineUnavailable => "engine_unavailable",
            Self::TorrentMetadataUnavailable => "torrent_metadata_unavailable",
            Self::InvalidUrl => "invalid_url",
            Self::Filesystem => "filesystem",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "network_unavailable" => Self::NetworkUnavailable,
            "timeout" => Self::Timeout,
            "server_rejected" => Self::ServerRejected,
            "not_found" => Self::NotFound,
            "permission_denied" => Self::PermissionDenied,
            "disk_full" => Self::DiskFull,
            "resume_not_supported" => Self::ResumeNotSupported,
            "extractor_failed" => Self::ExtractorFailed,
            "checksum_mismatch" => Self::ChecksumMismatch,
            "engine_unavailable" => Self::EngineUnavailable,
            "torrent_metadata_unavailable" => Self::TorrentMetadataUnavailable,
            "invalid_url" => Self::InvalidUrl,
            "filesystem" => Self::Filesystem,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Whether an automatic retry has a reasonable chance of succeeding.
    pub fn is_transient(self) -> bool {
        matches!(self, Self::NetworkUnavailable | Self::Timeout | Self::ServerRejected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Priority {
    Low,
    Normal,
    High,
}

impl Priority {
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
        }
    }
    pub fn from_i64(v: i64) -> Self {
        match v {
            i64::MIN..=0 => Self::Low,
            1 => Self::Normal,
            _ => Self::High,
        }
    }
}

/// Engine-specific options chosen when the download was added.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineOptions {
    /// yt-dlp format selector (e.g. `137+140` or `bestaudio`).
    #[serde(default)]
    #[ts(optional)]
    pub format_id: Option<String>,
    /// Extract audio only (requires FFmpeg for conversion).
    #[serde(default)]
    pub audio_only: bool,
    /// Torrent: indexes of the files to download (None = all).
    #[serde(default)]
    #[ts(optional)]
    pub torrent_files: Option<Vec<usize>>,
    /// HTTP: requested number of connections (None = use settings).
    #[serde(default)]
    #[ts(optional)]
    pub connections: Option<u32>,
    /// Audio-only: target format (best, mp3, m4a, opus, flac, wav).
    #[serde(default)]
    #[ts(optional)]
    pub audio_format: Option<String>,
    /// Expected SHA-256 of the finished file (64 hex chars); verified after download.
    #[serde(default)]
    #[ts(optional)]
    pub sha256: Option<String>,
    /// Video: download and embed subtitles.
    #[serde(default)]
    pub subtitles: bool,
    /// The user chose the filename; server-provided names must not replace it.
    #[serde(default)]
    pub explicit_filename: bool,
}

/// One persisted download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Download {
    pub id: String,
    /// The URL actually downloaded (after normalisation / redirects).
    pub url: String,
    /// The URL as supplied by the user or browser.
    pub original_url: String,
    pub referrer: Option<String>,
    pub filename: String,
    /// Directory the file is saved into.
    pub save_dir: String,
    pub category: String,
    pub engine: EngineKind,
    pub status: DownloadStatus,
    pub priority: Priority,
    #[ts(type = "number")]
    pub queue_position: i64,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    #[ts(type = "number")]
    pub downloaded_bytes: u64,
    #[ts(type = "number")]
    pub speed_bps: u64,
    #[ts(type = "number")]
    pub upload_bps: u64,
    #[ts(type = "number | null")]
    pub eta_seconds: Option<u64>,
    /// Whether the server/engine supports resuming. `None` = not yet known.
    pub resumable: Option<bool>,
    /// Per-download speed limit in bytes/s (`None` = unlimited).
    #[ts(type = "number | null")]
    pub speed_limit_bps: Option<u64>,
    pub connections: u32,
    pub error_kind: Option<ErrorKind>,
    pub error_message: Option<String>,
    pub error_detail: Option<String>,
    pub retry_count: u32,
    pub engine_options: EngineOptions,
    /// Torrent peers (live, total seen) — only real engine values.
    pub peers: Option<u32>,
    pub seeds: Option<u32>,
    pub info_hash: Option<String>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
    pub scheduled_at: Option<String>,
    /// Set when a completed download's file can no longer be found on disk.
    pub file_missing: bool,
    /// The queue this download belongs to ([`MAIN_QUEUE_ID`] by default).
    pub queue_id: String,
}

impl Download {
    pub fn full_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.save_dir).join(&self.filename)
    }
    pub fn progress(&self) -> f64 {
        match self.total_bytes {
            Some(t) if t > 0 => (self.downloaded_bytes as f64 / t as f64).clamp(0.0, 1.0),
            _ => {
                if self.status == DownloadStatus::Completed {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Request to add a download (from UI or browser extension).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddDownloadRequest {
    pub url: String,
    #[serde(default)]
    #[ts(optional)]
    pub filename: Option<String>,
    /// Destination directory. Must be inside an approved download directory.
    #[serde(default)]
    #[ts(optional)]
    pub save_dir: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub category: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<Priority>,
    /// Engine override; `None` = use detector.
    #[serde(default)]
    #[ts(optional)]
    pub engine: Option<EngineKind>,
    #[serde(default)]
    #[ts(optional)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub engine_options: EngineOptions,
    /// Start paused instead of queueing.
    #[serde(default)]
    pub start_paused: bool,
    /// ISO-8601 timestamp; the queue will not start the download earlier.
    #[serde(default)]
    #[ts(optional)]
    pub scheduled_at: Option<String>,
    /// Raw .torrent file content (base64) when adding a torrent file.
    #[serde(default)]
    #[ts(optional)]
    pub torrent_base64: Option<String>,
    /// Queue to add the download to (`None` = the main queue).
    #[serde(default)]
    #[ts(optional)]
    pub queue_id: Option<String>,
}

/// The built-in queue that holds every download not placed in another queue.
pub const MAIN_QUEUE_ID: &str = "main";

/// A download queue. Downloads in a queue start in priority / queue order while
/// both the global limit (`max_concurrent_downloads`) and the queue's own
/// `max_concurrent` allow it, and only while the queue is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Queue {
    pub id: String,
    /// Display name. Empty for the main queue until the user renames it (the UI
    /// then shows its translated default name).
    pub name: String,
    /// 1–20 downloads of this queue may run at the same time.
    pub max_concurrent: u32,
    /// A stopped queue starts nothing new.
    pub running: bool,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub created_at: String,
}

impl Queue {
    pub fn is_main(&self) -> bool {
        self.id == MAIN_QUEUE_ID
    }
}

/// Progress snapshot for one download, emitted in throttled batches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProgressUpdate {
    pub download_id: String,
    pub status: DownloadStatus,
    #[ts(type = "number")]
    pub downloaded_bytes: u64,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    #[ts(type = "number")]
    pub speed_bytes_per_second: u64,
    #[ts(type = "number")]
    pub upload_bytes_per_second: u64,
    #[ts(type = "number | null")]
    pub eta_seconds: Option<u64>,
    pub peers: Option<u32>,
    pub seeds: Option<u32>,
}

/// Events emitted by the manager. The Tauri shell forwards them to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ManagerEvent {
    DownloadCreated { download: Download },
    /// The full record changed (status, filename, error, ...).
    DownloadUpdated { download: Download },
    DownloadProgress { updates: Vec<ProgressUpdate> },
    DownloadCompleted { download: Download },
    DownloadFailed { download: Download },
    DownloadRemoved { id: String },
    /// A queue was created, renamed, started, stopped, changed or deleted.
    QueuesChanged { queues: Vec<Queue> },
    QueueFinished,
    /// A browser extension asked to open the Add dialog for this URL.
    ShowAddDialog { url: String },
    EngineError { engine: EngineKind, message: String },
}

/// A browser extension that has talked to the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionClient {
    /// chrome | edge | brave | firefox | chromium | opera | vivaldi | other
    pub browser: String,
    pub version: String,
    /// RFC 3339 time of the last message.
    pub last_seen: String,
}

/// File name / size / type of a link, looked up before sending it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkProbe {
    pub url: String,
    pub engine: EngineKind,
    pub filename: Option<String>,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    pub content_type: Option<String>,
    pub resumable: Option<bool>,
    /// Classified failure (e.g. not_found); the link can still be sent.
    pub error: Option<String>,
}

/// Global transfer statistics for the status bar / tray.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GlobalStats {
    pub total: u32,
    pub active: u32,
    pub queued: u32,
    pub completed: u32,
    pub failed: u32,
    #[ts(type = "number")]
    pub download_bps: u64,
    #[ts(type = "number")]
    pub upload_bps: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use DownloadStatus::*;

    #[test]
    fn valid_transitions() {
        assert!(Queued.can_transition_to(Preparing));
        assert!(Preparing.can_transition_to(Downloading));
        assert!(Downloading.can_transition_to(Completed));
        assert!(Downloading.can_transition_to(Paused));
        assert!(Paused.can_transition_to(Queued));
        assert!(Failed.can_transition_to(Queued));
        assert!(Cancelled.can_transition_to(Queued));
    }

    #[test]
    fn invalid_transitions() {
        assert!(!Completed.can_transition_to(Downloading));
        assert!(!Completed.can_transition_to(Paused));
        assert!(!Paused.can_transition_to(Downloading));
        assert!(!Cancelled.can_transition_to(Completed));
        assert!(!Failed.can_transition_to(Completed));
        assert!(!Queued.can_transition_to(Completed));
        assert!(!Queued.can_transition_to(Downloading));
        for s in [Queued, Preparing, Downloading, Paused, Completed, Failed, Cancelled] {
            assert!(!s.can_transition_to(s), "{s} -> {s} must be rejected");
        }
    }

    #[test]
    fn status_round_trip() {
        for s in [Queued, Preparing, Downloading, Paused, Completed, Failed, Cancelled] {
            assert_eq!(DownloadStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(DownloadStatus::parse("bogus"), None);
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::High > Priority::Normal);
        assert_eq!(Priority::from_i64(Priority::High.as_i64()), Priority::High);
        assert_eq!(Priority::from_i64(-5), Priority::Low);
    }
}
