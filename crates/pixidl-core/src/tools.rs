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
            // Also `<dir>/<tool>/<tool>.exe`: bundles with libraries (FFmpeg) get their own folder.
            for p in [d.join(&name), d.join(base).join(&name)] {
                if is_executable(&p) {
                    return Some(p);
                }
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

/// Official yt-dlp release asset for this platform.
pub fn ytdlp_asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("yt-dlp.exe")
    } else if cfg!(all(windows, target_arch = "x86")) {
        Some("yt-dlp_x86.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("yt-dlp_linux")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("yt-dlp_linux_aarch64")
    } else if cfg!(target_os = "macos") {
        Some("yt-dlp_macos")
    } else {
        None
    }
}

/// Finds the hash for `asset` in a `SHA2-256SUMS` file.
pub fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

pub const YTDLP_RELEASE_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

/// Installs yt-dlp from the official GitHub release into `dest_dir`, verifying
/// the SHA-256 checksum published with the release. Only ever run on an
/// explicit user action.
pub async fn install_ytdlp(client: &reqwest::Client, dest_dir: &Path) -> crate::Result<PathBuf> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    use sha2::{Digest, Sha256};
    let asset = ytdlp_asset_name().ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "No official yt-dlp build for this platform"))?;
    let get = |url: String| async move {
        let r = client.get(&url).send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
        if !r.status().is_success() {
            return Err(DownloadError::from_status(r.status().as_u16()));
        }
        r.bytes().await.map_err(|e| DownloadError::from_reqwest(&e))
    };
    let sums = get(format!("{YTDLP_RELEASE_BASE}/SHA2-256SUMS")).await?;
    let expected = checksum_for(&String::from_utf8_lossy(&sums), asset)
        .ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Release checksum not found"))?;
    let bin = get(format!("{YTDLP_RELEASE_BASE}/{asset}")).await?;
    let actual = hex::encode(Sha256::digest(&bin));
    if actual != expected {
        return Err(DownloadError::new(ErrorKind::EngineUnavailable, "Downloaded yt-dlp failed checksum verification").with_detail(format!("expected {expected}, got {actual}")));
    }
    tokio::fs::create_dir_all(dest_dir).await.map_err(|e| DownloadError::fs("Cannot create engines folder", &e))?;
    let target = dest_dir.join(exe_name("yt-dlp"));
    let tmp = dest_dir.join(format!("{}.download", exe_name("yt-dlp")));
    tokio::fs::write(&tmp, &bin).await.map_err(|e| DownloadError::fs("Cannot write yt-dlp", &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    tokio::fs::rename(&tmp, &target).await.map_err(|e| DownloadError::fs("Cannot install yt-dlp", &e))?;
    tracing::info!(path = %target.display(), "installed yt-dlp from official release");
    Ok(target)
}

#[cfg(test)]
mod install_tests {
    #[test]
    fn parses_checksums() {
        let sums = "aa  other\n0123456789abcdef0123456789abcdef0123456789abcdef0123456789ABCDEF  yt-dlp.exe\nbad  yt-dlp_linux\n";
        assert_eq!(super::checksum_for(sums, "yt-dlp.exe").unwrap(), "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        assert!(super::checksum_for(sums, "yt-dlp_linux").is_none());
        assert!(super::checksum_for(sums, "missing").is_none());
    }
}

/// Official Deno release archive for this platform.
pub fn deno_asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("deno-x86_64-pc-windows-msvc.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("deno-x86_64-unknown-linux-gnu.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("deno-aarch64-unknown-linux-gnu.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("deno-aarch64-apple-darwin.zip")
    } else if cfg!(target_os = "macos") {
        Some("deno-x86_64-apple-darwin.zip")
    } else {
        None
    }
}

pub const DENO_RELEASE_BASE: &str = "https://github.com/denoland/deno/releases/latest/download";

/// Extracts the single executable from a release zip (only entries named
/// `name`; never writes outside `dest_dir`).
fn extract_exe_from_zip(bytes: &[u8], name: &str, dest_dir: &Path) -> crate::Result<PathBuf> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| DownloadError::new(ErrorKind::EngineUnavailable, "Invalid archive").with_detail(e.to_string()))?;
    let mut file = zip.by_name(name).map_err(|e| DownloadError::new(ErrorKind::EngineUnavailable, "Archive does not contain the executable").with_detail(e.to_string()))?;
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf).map_err(|e| DownloadError::fs("Cannot read archive", &e))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| DownloadError::fs("Cannot create engines folder", &e))?;
    let target = dest_dir.join(name);
    let tmp = dest_dir.join(format!("{name}.download"));
    std::fs::write(&tmp, &buf).map_err(|e| DownloadError::fs("Cannot write executable", &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, &target).map_err(|e| DownloadError::fs("Cannot install executable", &e))?;
    Ok(target)
}

/// Installs Deno (MIT) from the official GitHub release after verifying the
/// published SHA-256. yt-dlp uses it to run YouTube's player JavaScript.
/// Only ever run on an explicit user action.
pub async fn install_deno(client: &reqwest::Client, dest_dir: &Path) -> crate::Result<PathBuf> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    use sha2::{Digest, Sha256};
    let asset = deno_asset_name().ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "No official Deno build for this platform"))?;
    let get = |url: String| async move {
        let r = client.get(&url).send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
        if !r.status().is_success() {
            return Err(DownloadError::from_status(r.status().as_u16()));
        }
        r.bytes().await.map_err(|e| DownloadError::from_reqwest(&e))
    };
    let sums = get(format!("{DENO_RELEASE_BASE}/{asset}.sha256sum")).await?;
    let sums = String::from_utf8_lossy(&sums);
    // Either "<hash>  <file>" or a bare hash.
    let expected = checksum_for(&sums, asset)
        .or_else(|| sums.split_whitespace().next().filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).map(|h| h.to_ascii_lowercase()))
        .ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Release checksum not found"))?;
    let archive = get(format!("{DENO_RELEASE_BASE}/{asset}")).await?;
    let actual = hex::encode(Sha256::digest(&archive));
    if actual != expected {
        return Err(DownloadError::new(ErrorKind::EngineUnavailable, "Downloaded Deno failed checksum verification").with_detail(format!("expected {expected}, got {actual}")));
    }
    let target = extract_exe_from_zip(&archive, &exe_name("deno"), dest_dir)?;
    tracing::info!(path = %target.display(), "installed Deno from official release");
    Ok(target)
}

/// Status of the JavaScript runtime yt-dlp will use.
pub async fn js_runtime_status(loc: &ToolLocator, configured: &str) -> ToolStatus {
    match crate::engines::video::find_js_runtime(loc, configured) {
        None => ToolStatus {
            name: "js-runtime".into(),
            available: false,
            path: None,
            version: None,
            message: Some("No JavaScript runtime found. YouTube downloads need Deno (recommended), Node.js or Bun.".into()),
        },
        Some((name, p)) => {
            let v = version_of(&p, &["--version"]).await.map(|l| {
                let l = l.trim_start_matches("deno ").trim_start_matches('v').to_string();
                format!("{name} {}", l.split_whitespace().next().unwrap_or(&l))
            });
            ToolStatus { name: name.into(), available: v.is_some(), path: Some(p.display().to_string()), version: v, message: None }
        }
    }
}

#[cfg(test)]
mod zip_tests {
    #[test]
    fn extracts_only_the_named_entry() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("deno", o).unwrap();
            w.write_all(b"binary").unwrap();
            w.start_file("../evil", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let d = tempfile::tempdir().unwrap();
        let p = super::extract_exe_from_zip(buf.get_ref(), "deno", d.path()).unwrap();
        assert_eq!(std::fs::read(p).unwrap(), b"binary");
        assert!(!d.path().parent().unwrap().join("evil").exists());
        assert!(super::extract_exe_from_zip(buf.get_ref(), "missing", d.path()).is_err());
    }
}

/// Progress of an automatic tool download (bytes so far, total if known).
pub type InstallProgress<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// Streams `url` into `dest`, hashing as it goes; the file is removed unless
/// its SHA-256 equals `expected`.
async fn download_verified(client: &reqwest::Client, url: &str, expected: &str, dest: &Path, progress: InstallProgress<'_>) -> crate::Result<()> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    use futures::StreamExt;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;
    let r = client.get(url).send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    if !r.status().is_success() {
        return Err(DownloadError::from_status(r.status().as_u16()));
    }
    let total = r.content_length();
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| DownloadError::fs("Cannot write download", &e))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    let mut stream = r.bytes_stream();
    let result = async {
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), stream.next()).await.map_err(|_| DownloadError::new(ErrorKind::Timeout, "Download stalled"))? {
            let chunk = chunk.map_err(|e| DownloadError::from_reqwest(&e))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| DownloadError::fs("Cannot write download", &e))?;
            done += chunk.len() as u64;
            if last.elapsed() >= Duration::from_millis(250) {
                last = std::time::Instant::now();
                progress(done, total);
            }
        }
        file.flush().await.map_err(|e| DownloadError::fs("Cannot write download", &e))?;
        progress(done, total);
        let actual = hex::encode(hasher.finalize());
        if actual != expected {
            return Err(DownloadError::new(ErrorKind::EngineUnavailable, "Downloaded file failed checksum verification").with_detail(format!("expected {expected}, got {actual}")));
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// FFmpeg build published by the yt-dlp project (GPL, with the patches yt-dlp
/// relies on). The "shared" build keeps the download at ~90 MB instead of 200.
pub fn ffmpeg_asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("ffmpeg-master-latest-win64-gpl-shared.zip")
    } else {
        None
    }
}

pub const FFMPEG_RELEASE_BASE: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest";

/// Copies `bin/ffmpeg.exe`, `bin/ffprobe.exe` and the `bin/*.dll` libraries
/// out of an FFmpeg-Builds zip into `dest_dir` (flat; never outside it).
pub fn extract_ffmpeg_bundle(zip_path: &Path, dest_dir: &Path) -> crate::Result<PathBuf> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    let bad = |e: String| DownloadError::new(ErrorKind::EngineUnavailable, "Invalid FFmpeg archive").with_detail(e);
    let f = std::fs::File::open(zip_path).map_err(|e| DownloadError::fs("Cannot open archive", &e))?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| bad(e.to_string()))?;
    std::fs::create_dir_all(dest_dir).map_err(|e| DownloadError::fs("Cannot create engines folder", &e))?;
    let mut found_ffmpeg = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else { continue };
        let parent = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_ascii_lowercase());
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
        let lower = name.to_ascii_lowercase();
        let wanted = parent.as_deref() == Some("bin") && (lower == "ffmpeg.exe" || lower == "ffprobe.exe" || lower.ends_with(".dll"));
        if !wanted {
            continue;
        }
        let mut out = std::fs::File::create(dest_dir.join(&name)).map_err(|e| DownloadError::fs("Cannot write FFmpeg", &e))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| DownloadError::fs("Cannot write FFmpeg", &e))?;
        found_ffmpeg |= lower == "ffmpeg.exe";
    }
    if !found_ffmpeg {
        return Err(bad("bin/ffmpeg.exe not found".into()));
    }
    Ok(dest_dir.join("ffmpeg.exe"))
}

/// Installs FFmpeg (checksum-verified) into `dest_dir`, replacing an older copy.
pub async fn install_ffmpeg(client: &reqwest::Client, dest_dir: &Path, progress: InstallProgress<'_>) -> crate::Result<PathBuf> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    let asset = ffmpeg_asset_name().ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Automatic FFmpeg setup is only available on Windows").with_detail("Install FFmpeg with your package manager."))?;
    let sums = client.get(format!("{FFMPEG_RELEASE_BASE}/checksums.sha256")).send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    if !sums.status().is_success() {
        return Err(DownloadError::from_status(sums.status().as_u16()));
    }
    let sums = sums.text().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    let expected = checksum_for(&sums, asset).ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Release checksum not found"))?;
    let parent = dest_dir.parent().unwrap_or(dest_dir).to_path_buf();
    tokio::fs::create_dir_all(&parent).await.map_err(|e| DownloadError::fs("Cannot create engines folder", &e))?;
    let archive = parent.join(format!("{asset}.download"));
    download_verified(client, &format!("{FFMPEG_RELEASE_BASE}/{asset}"), &expected, &archive, progress).await?;
    let staging = parent.join("ffmpeg.new");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let (a, s) = (archive.clone(), staging.clone());
    let extracted = tokio::task::spawn_blocking(move || extract_ffmpeg_bundle(&a, &s)).await.map_err(|e| DownloadError::new(ErrorKind::Unknown, "FFmpeg setup crashed").with_detail(e.to_string()))?;
    let _ = tokio::fs::remove_file(&archive).await;
    extracted?;
    let _ = tokio::fs::remove_dir_all(dest_dir).await;
    tokio::fs::rename(&staging, dest_dir).await.map_err(|e| DownloadError::fs("Cannot install FFmpeg", &e))?;
    let exe = dest_dir.join("ffmpeg.exe");
    tracing::info!(path = %exe.display(), "installed FFmpeg from yt-dlp/FFmpeg-Builds");
    Ok(exe)
}

/// Whether the yt-dlp binary at `local` differs from the latest official
/// release (compares its SHA-256 with the published SHA2-256SUMS).
pub async fn ytdlp_is_outdated(client: &reqwest::Client, local: &Path) -> crate::Result<bool> {
    use crate::error::DownloadError;
    use crate::types::ErrorKind;
    use sha2::{Digest, Sha256};
    let asset = ytdlp_asset_name().ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "No official yt-dlp build for this platform"))?;
    let r = client.get(format!("{YTDLP_RELEASE_BASE}/SHA2-256SUMS")).send().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    if !r.status().is_success() {
        return Err(DownloadError::from_status(r.status().as_u16()));
    }
    let sums = r.text().await.map_err(|e| DownloadError::from_reqwest(&e))?;
    let expected = checksum_for(&sums, asset).ok_or_else(|| DownloadError::new(ErrorKind::EngineUnavailable, "Release checksum not found"))?;
    let local = local.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || std::fs::read(&local).map(|b| hex::encode(Sha256::digest(&b))))
        .await
        .map_err(|e| DownloadError::new(ErrorKind::Unknown, "Hashing failed").with_detail(e.to_string()))?
        .map_err(|e| DownloadError::fs("Cannot read yt-dlp", &e))?;
    Ok(actual != expected)
}

#[cfg(test)]
mod ffmpeg_tests {
    use std::io::Write;

    #[test]
    fn extracts_binaries_and_libraries_only() {
        let d = tempfile::tempdir().unwrap();
        let zip_path = d.path().join("ff.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("ffmpeg-master-latest-win64-gpl-shared/bin/ffmpeg.exe", "ff"),
                ("ffmpeg-master-latest-win64-gpl-shared/bin/ffprobe.exe", "fp"),
                ("ffmpeg-master-latest-win64-gpl-shared/bin/avcodec-62.dll", "dll"),
                ("ffmpeg-master-latest-win64-gpl-shared/bin/ffplay.exe", "no"),
                ("ffmpeg-master-latest-win64-gpl-shared/doc/readme.txt", "no"),
                ("../bin/evil.dll", "no"),
            ] {
                w.start_file(name, o).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let out = d.path().join("ffmpeg");
        let exe = super::extract_ffmpeg_bundle(&zip_path, &out).unwrap();
        assert_eq!(std::fs::read(exe).unwrap(), b"ff");
        let mut names: Vec<_> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["avcodec-62.dll", "ffmpeg.exe", "ffprobe.exe"]);
        assert!(!d.path().join("bin").exists());
    }

    #[test]
    fn locator_finds_tools_in_their_own_folder() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("ffmpeg");
        std::fs::create_dir_all(&sub).unwrap();
        let exe = sub.join(super::exe_name("ffmpeg"));
        std::fs::write(&exe, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let loc = super::ToolLocator::without_system_path(vec![d.path().to_path_buf()]);
        assert_eq!(loc.find("ffmpeg", ""), Some(exe));
    }
}
