//! App updates (desktop side of `pixidl_core::updater`): the daily background
//! check, the download with progress events, and running the verified
//! installer.
//!
//! Installing runs the NSIS installer Tauri generates with `/P /R /UPDATE`:
//! - `/P` passive: no pages or prompts, only a progress window; a running
//!   pixidl is closed through the Restart Manager instead of asking the user
//! - `/R` relaunch pixidl when the install succeeds (passive/silent only)
//! - `/UPDATE` update in place: no uninstall step, no WebView2 bootstrapper,
//!   shortcuts the user removed are not re-created
//!
//! The app pauses its transfers and exits right after starting the installer.

use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use pixidl_core::types::ErrorKind;
use pixidl_core::updater::{self, CancellationToken, UpdateEvent, UpdateInfo, Updater};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;

pub const UPDATE_CHANNEL: &str = "pixidl://update";
/// Flags understood by Tauri's NSIS installer template (see the module docs).
pub const INSTALLER_ARGS: [&str; 3] = ["/P", "/R", "/UPDATE"];

pub struct ReadyInstaller {
    pub path: PathBuf,
    pub sha256: String,
    pub version: String,
}

#[derive(Default)]
pub struct UpdaterState {
    pub info: Mutex<Option<UpdateInfo>>,
    pub download: Mutex<Option<CancellationToken>>,
    pub ready: Mutex<Option<ReadyInstaller>>,
    /// Serialises checks (background and manual).
    pub checking: tokio::sync::Mutex<()>,
}

/// Everything the Updates page needs to render.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub info: Option<UpdateInfo>,
    pub last_checked: Option<String>,
    pub downloading: bool,
    pub ready_version: Option<String>,
    /// The installer can be run from inside the app (Windows).
    pub can_install: bool,
    pub repo_url: String,
    pub releases_url: String,
}

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

pub fn status(app: &AppHandle) -> UpdateStatus {
    let st = app.state::<UpdaterState>();
    let last_checked = app.try_state::<AppState>().and_then(|s| s.mgr.db().get_kv(updater::LAST_CHECK_KEY).ok().flatten());
    let info = st.info.lock().clone();
    let downloading = st.download.lock().is_some();
    let ready_version = st.ready.lock().as_ref().map(|r| r.version.clone());
    UpdateStatus {
        current_version: current_version(app),
        info,
        last_checked,
        downloading,
        ready_version,
        can_install: updater::PLATFORM_SUPPORTED,
        repo_url: updater::REPO_URL.into(),
        releases_url: updater::RELEASES_URL.into(),
    }
}

fn client(app: &AppHandle) -> CmdResult<Updater> {
    let s = app.state::<AppState>().mgr.settings();
    Ok(Updater::github(&s)?)
}

/// Checks GitHub now and remembers the result and the time.
pub async fn check(app: &AppHandle) -> CmdResult<UpdateInfo> {
    let st = app.state::<UpdaterState>();
    let _guard = st.checking.lock().await;
    let include_pre = app.state::<AppState>().mgr.settings().include_prereleases;
    let info = client(app)?.check(&current_version(app), include_pre).await?;
    let _ = app.state::<AppState>().mgr.db().set_kv(updater::LAST_CHECK_KEY, &updater::now_rfc3339());
    // A different release than the one already downloaded invalidates it.
    {
        let mut ready = st.ready.lock();
        if ready.as_ref().is_some_and(|r| info.latest.as_ref().map(|l| &l.version) != Some(&r.version)) {
            *ready = None;
        }
    }
    *st.info.lock() = Some(info.clone());
    Ok(info)
}

/// Downloads and verifies the installer of the release found by the last check.
pub async fn download(app: &AppHandle) -> CmdResult<()> {
    let st = app.state::<UpdaterState>();
    let info = st.info.lock().clone().ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "Check for updates first"))?;
    let latest = info.latest.clone().filter(|_| info.update_available).ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "No update is available"))?;
    if let Some(b) = info.install_blocker {
        return Err(CommandError::msg(ErrorKind::EngineUnavailable, match b {
            updater::InstallBlocker::UnsupportedPlatform => "Automatic installation is only available on Windows; download the update from the release page",
            updater::InstallBlocker::NoInstaller => "This release has no Windows installer",
            updater::InstallBlocker::NoChecksum => "This release has no published checksum, so it can't be installed automatically",
        }));
    }
    let asset = latest.installer.clone().ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "This release has no Windows installer"))?;
    let token = {
        let mut slot = st.download.lock();
        if slot.is_some() {
            return Err(CommandError::msg(ErrorKind::Unknown, "The update is already downloading"));
        }
        let t = CancellationToken::new();
        *slot = Some(t.clone());
        t
    };
    *st.ready.lock() = None;
    let up = client(app);
    let emitter = app.clone();
    let result = match up {
        Ok(up) => up
            .download(&asset, latest.sha256.as_deref(), &updater::updates_dir(), &move |e: UpdateEvent| { let _ = emitter.emit(UPDATE_CHANNEL, &e); }, &token)
            .await
            .map_err(CommandError::from),
        Err(e) => Err(e),
    };
    *st.download.lock() = None;
    let path = result?;
    let sha256 = latest.sha256.clone().unwrap_or_default();
    tracing::info!(version = %latest.version, path = %path.display(), "update downloaded and verified");
    *st.ready.lock() = Some(ReadyInstaller { path, sha256, version: latest.version.clone() });
    let _ = app.emit(UPDATE_CHANNEL, &UpdateEvent::Ready { version: latest.version });
    Ok(())
}

pub fn cancel_download(app: &AppHandle) -> bool {
    match app.state::<UpdaterState>().download.lock().as_ref() {
        Some(t) => {
            t.cancel();
            true
        }
        None => false,
    }
}

/// Runs the verified installer and exits so it can replace the files.
pub fn install(app: &AppHandle) -> CmdResult<()> {
    if !updater::PLATFORM_SUPPORTED {
        return Err(CommandError::msg(ErrorKind::EngineUnavailable, "Automatic installation is only available on Windows; download the update from the release page"));
    }
    let st = app.state::<UpdaterState>();
    let (path, sha256, version) = {
        let ready = st.ready.lock();
        let r = ready.as_ref().ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "Download the update first"))?;
        (r.path.clone(), r.sha256.clone(), r.version.clone())
    };
    // Never execute a file that doesn't match the published checksum, even
    // if it changed on disk after the download was verified.
    if let Err(e) = updater::verify_file(&path, &sha256) {
        *st.ready.lock() = None;
        let _ = std::fs::remove_file(&path);
        return Err(e.into());
    }
    tracing::info!(%version, path = %path.display(), args = ?INSTALLER_ARGS, "starting the update installer");
    std::process::Command::new(&path)
        .args(INSTALLER_ARGS)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| CommandError { kind: ErrorKind::EngineUnavailable, message: "Could not start the installer".into(), detail: Some(e.to_string()) })?;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    crate::quit(app.clone());
    Ok(())
}

/// Opens a page of the project on GitHub (release notes, releases, repository).
pub fn open_page(app: &AppHandle, url: Option<String>) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = url.unwrap_or_else(|| updater::RELEASES_URL.into());
    if !(url == updater::REPO_URL || url.starts_with(&format!("{}/", updater::REPO_URL))) {
        return Err(CommandError::msg(ErrorKind::PermissionDenied, "Only pages of the pixidl project can be opened"));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| CommandError { kind: ErrorKind::Unknown, message: "Could not open the browser".into(), detail: Some(e.to_string()) })
}

/// Startup: removes installers left from a previous update, then checks
/// GitHub once a day while automatic checks are enabled.
pub fn spawn_background_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let dir = updater::updates_dir();
        let _ = tokio::task::spawn_blocking(move || updater::clear_downloads(&dir)).await;
        // Let the window and the queue start first.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            background_check(&app).await;
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    });
}

async fn background_check(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if !state.mgr.settings().auto_check_updates || state.quitting.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let last = state.mgr.db().get_kv(updater::LAST_CHECK_KEY).ok().flatten();
    if !updater::check_due_now(last.as_deref()) {
        return;
    }
    match check(app).await {
        Ok(info) if info.update_available => {
            let version = info.latest.as_ref().map(|l| l.version.clone()).unwrap_or_default();
            tracing::info!(%version, "update available");
            let _ = app.emit(UPDATE_CHANNEL, &UpdateEvent::Available { info: Box::new(info) });
            crate::notify(app, "Update available", &format!("pixidl {version} is available"));
        }
        Ok(_) => tracing::debug!("no update available"),
        Err(e) => tracing::info!(error = %e.message, detail = ?e.detail, "automatic update check failed"),
    }
}
