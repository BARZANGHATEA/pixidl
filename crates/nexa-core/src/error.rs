//! Error types. Every failure is classified into an [`ErrorKind`] with a
//! human-readable message and optional technical detail (shown behind "Details").

use crate::types::ErrorKind;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct DownloadError {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: Option<String>,
}

impl DownloadError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), detail: None }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn invalid_url(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidUrl, msg)
    }

    pub fn fs(msg: impl Into<String>, e: &std::io::Error) -> Self {
        Self::from_io(e).with_message(msg)
    }

    fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = msg.into();
        self
    }

    pub fn from_io(e: &std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let kind = match e.kind() {
            K::PermissionDenied => ErrorKind::PermissionDenied,
            K::StorageFull => ErrorKind::DiskFull,
            K::NotFound => ErrorKind::Filesystem,
            K::TimedOut => ErrorKind::Timeout,
            _ => {
                // ENOSPC (28) on unix, ERROR_DISK_FULL (112) / ERROR_HANDLE_DISK_FULL (39) on Windows.
                match e.raw_os_error() {
                    #[cfg(unix)]
                    Some(28) => ErrorKind::DiskFull,
                    #[cfg(windows)]
                    Some(112) | Some(39) => ErrorKind::DiskFull,
                    _ => ErrorKind::Filesystem,
                }
            }
        };
        let message = match kind {
            ErrorKind::PermissionDenied => "Permission denied",
            ErrorKind::DiskFull => "Disk full",
            ErrorKind::Timeout => "Connection timed out",
            _ => "File system error",
        };
        Self::new(kind, message).with_detail(e.to_string())
    }

    pub fn from_reqwest(e: &reqwest::Error) -> Self {
        let detail = redact(&format!("{e:#}"));
        if e.is_timeout() {
            return Self::new(ErrorKind::Timeout, "Connection timed out").with_detail(detail);
        }
        if e.is_connect() || e.is_request() {
            return Self::new(ErrorKind::NetworkUnavailable, "Network unavailable").with_detail(detail);
        }
        if e.is_body() || e.is_decode() {
            return Self::new(ErrorKind::NetworkUnavailable, "Connection interrupted").with_detail(detail);
        }
        if e.is_redirect() {
            return Self::new(ErrorKind::ServerRejected, "Too many redirects").with_detail(detail);
        }
        if let Some(status) = e.status() {
            return Self::from_status(status.as_u16()).with_detail(detail);
        }
        Self::new(ErrorKind::NetworkUnavailable, "Network error").with_detail(detail)
    }

    pub fn from_status(status: u16) -> Self {
        match status {
            404 | 410 => Self::new(ErrorKind::NotFound, "File not found"),
            401 | 403 => Self::new(ErrorKind::ServerRejected, "Server rejected the request (access denied)"),
            416 => Self::new(ErrorKind::ResumeNotSupported, "Server rejected the resume range"),
            429 => Self::new(ErrorKind::ServerRejected, "Server is rate limiting requests"),
            500..=599 => Self::new(ErrorKind::ServerRejected, "Temporary server failure"),
            _ => Self::new(ErrorKind::ServerRejected, "Server rejected the request"),
        }
        .with_detail(format!("HTTP status {status}"))
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Download cancelled")
    }

    /// Whether the retry policy should retry this error automatically.
    pub fn is_retryable(&self) -> bool {
        match self.kind {
            ErrorKind::NetworkUnavailable | ErrorKind::Timeout => true,
            ErrorKind::ServerRejected => {
                // Retry 5xx / 429 but not 401/403.
                self.detail.as_deref().map_or(false, |d| {
                    d.contains("HTTP status 5") || d.contains("HTTP status 429")
                })
            }
            _ => false,
        }
    }
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        Self::from_io(&e)
    }
}

impl From<rusqlite::Error> for DownloadError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(ErrorKind::Unknown, "Database error").with_detail(e.to_string())
    }
}

/// Removes credentials and query strings from text before it is logged or stored.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, token) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if let Ok(mut u) = url::Url::parse(token.trim_matches(|c| c == '(' || c == ')' || c == '"')) {
            if u.has_host() {
                let _ = u.set_password(None);
                if !u.username().is_empty() {
                    let _ = u.set_username("***");
                }
                if u.query().is_some() {
                    u.set_query(Some("…"));
                }
                out.push_str(u.as_str());
                continue;
            }
        }
        out.push_str(token);
    }
    out
}

/// Redacts a single URL for logging.
pub fn redact_url(u: &str) -> String {
    redact(u)
}

pub type Result<T, E = DownloadError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_and_query() {
        let r = redact("failed for https://user:secret@example.com/a.zip?token=abc now");
        assert!(!r.contains("secret"));
        assert!(!r.contains("token=abc"));
        assert!(r.contains("example.com/a.zip"));
        assert!(r.starts_with("failed for "));
    }

    #[test]
    fn status_classification() {
        assert_eq!(DownloadError::from_status(404).kind, ErrorKind::NotFound);
        assert!(DownloadError::from_status(503).is_retryable());
        assert!(DownloadError::from_status(429).is_retryable());
        assert!(!DownloadError::from_status(403).is_retryable());
        assert!(!DownloadError::from_status(404).is_retryable());
    }

    #[test]
    fn io_classification() {
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(DownloadError::from_io(&e).kind, ErrorKind::PermissionDenied);
    }
}
