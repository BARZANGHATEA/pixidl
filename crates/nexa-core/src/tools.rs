//! Locating, validating and versioning external tools (yt-dlp, FFmpeg).
//!
//! Search order for each tool:
//! 1. the path configured in Settings → Engines (if it exists)
//! 2. directories supplied by the shell (bundled `resources/bin`, app-data `engines/`)
//! 3. the system `PATH`
//!
//! Executables are always started with argument arrays (never a shell), and
//! without a console window on Windows.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolStatus {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolLocator {
    search_dirs: Vec<PathBuf>,
    skip_system_path: bool,
}

pub fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn is_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

impl ToolLocator {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self { search_dirs, skip_system_path: false }
    }

    /// Only look in the configured path and the given directories.
    pub fn without_system_path(search_dirs: Vec<PathBuf>) -> Self {
        Self { search_dirs, skip_system_path: true }
    }

    pub fn search_dirs(&self) -> &[PathBuf] {
        &self.search_dirs
    }

    pub fn find(&self, base: &str, configured: &str) -> Option<PathBuf> {
        let configured = configured.trim();
        if !configured.is_empty() {
            let p = PathBuf::from(configured);
            if is_executable(&p) {
                return Some(p);
            }
        }
        let name = exe_name(base);
        for d in &self.search_dirs {
            let p = d.join(&name);
            if is_executable(&p) {
                return Some(p);
            }
        }
        if self.skip_system_path {
            return None;
        }
        let path_var = std::env::var_os("PATH")?;
        std::env::split_paths(&path_var).map(|d| d.join(&name)).find(|p| is_executable(p))
    }
}

/// Builds a command that never opens a console window on Windows.
pub fn command(program: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Runs `program args…` and returns the first line of stdout.
pub async fn version_of(program: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(Duration::from_secs(15), command(program).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

pub async fn ytdlp_status(loc: &ToolLocator, configured: &str) -> ToolStatus {
    match loc.find("yt-dlp", configured) {
        None => ToolStatus {
            name: "yt-dlp".into(),
            available: false,
            path: None,
            version: None,
            message: Some("yt-dlp was not found. Install it or set its path in Settings → Engines.".into()),
        },
        Some(p) => {
            let v = version_of(&p, &["--version"]).await;
            ToolStatus {
                name: "yt-dlp".into(),
                available: v.is_some(),
                message: v.is_none().then(|| "yt-dlp was found but could not be executed".to_string()),
                path: Some(p.display().to_string()),
                version: v,
            }
        }
    }
}

pub async fn ffmpeg_status(loc: &ToolLocator, configured: &str) -> ToolStatus {
    match loc.find("ffmpeg", configured) {
        None => ToolStatus {
            name: "ffmpeg".into(),
            available: false,
            path: None,
            version: None,
            message: Some("FFmpeg was not found. Video+audio merging and audio extraction are unavailable.".into()),
        },
        Some(p) => {
            let v = version_of(&p, &["-version"]).await.map(|l| {
                // "ffmpeg version 6.1.1-3ubuntu5 Copyright ..." → "6.1.1-3ubuntu5"
                l.split_whitespace().nth(2).unwrap_or(&l).to_string()
            });
            ToolStatus {
                name: "ffmpeg".into(),
                available: v.is_some(),
                message: None,
                path: Some(p.display().to_string()),
                version: v,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_path_must_exist() {
        let loc = ToolLocator::new(vec![]);
        assert!(loc.find("definitely-not-a-real-tool-xyz", "/nope/nothing").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn finds_in_search_dirs() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("faketool");
        std::fs::write(&p, "#!/bin/sh\necho 1.2.3\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let loc = ToolLocator::new(vec![d.path().to_path_buf()]);
        assert_eq!(loc.find("faketool", ""), Some(p.clone()));
        // Non-executable files are ignored.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(loc.find("faketool", "").is_none());
    }
}
