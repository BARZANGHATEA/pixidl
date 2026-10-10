//! Typed IPC commands exposed to the frontend. Each maps 1:1 to a manager or
//! shell operation; inputs are validated by the core.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use pixidl_core::browser::{self, BrowserRegistration};
use pixidl_core::db::{Category, DownloadEvent};
use pixidl_core::detector::UrlInspection;
use pixidl_core::engines::torrent::{TorrentEngineStatus, TorrentInfo};
use pixidl_core::manager::AddSource;
use pixidl_core::settings::Settings;
use pixidl_core::tools::ToolStatus;
use pixidl_core::types::*;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt;

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> CmdResult<Vec<Download>> {
    Ok(state.mgr.list()?)
}

#[tauri::command]
pub fn get_download(state: State<'_, AppState>, id: String) -> CmdResult<Option<Download>> {
    Ok(state.mgr.get(&id)?)
}

#[tauri::command]
pub fn get_download_events(state: State<'_, AppState>, id: String) -> CmdResult<Vec<DownloadEvent>> {
    Ok(state.mgr.events(&id)?)
}

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> CmdResult<GlobalStats> {
    Ok(state.mgr.stats()?)
}

#[tauri::command]
pub async fn add_download(state: State<'_, AppState>, request: AddDownloadRequest) -> CmdResult<Download> {
    Ok(state.mgr.add(request, AddSource::User).await?)
}

#[tauri::command]
pub async fn inspect_url(state: State<'_, AppState>, url: String, engine: Option<EngineKind>) -> CmdResult<UrlInspection> {
    Ok(state.mgr.inspect_url(&url, engine).await?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFilePreview {
    pub info: TorrentInfo,
    pub base64: String,
    pub file_name: String,
}

/// Reads a `.torrent` the user picked in the file dialog.
#[tauri::command]
pub async fn read_torrent_file(state: State<'_, AppState>, path: String) -> CmdResult<TorrentFilePreview> {
    let p = PathBuf::from(&path);
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent")) {
        return Err(CommandError::msg(ErrorKind::InvalidUrl, "Please choose a .torrent file"));
    }
    let meta = std::fs::metadata(&p).map_err(|e| CommandError::from(pixidl_core::DownloadError::from_io(&e)))?;
    if meta.len() > 16 * 1024 * 1024 {
        return Err(CommandError::msg(ErrorKind::InvalidUrl, "Torrent file is too large"));
    }
    let bytes = std::fs::read(&p).map_err(|e| CommandError::from(pixidl_core::DownloadError::from_io(&e)))?;
    let info = state.mgr.inspect_torrent_file(bytes.clone()).await?;
    Ok(TorrentFilePreview {
        info,
        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        file_name: p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    })
}

#[tauri::command]
pub fn pause_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.mgr.pause(&id)?)
}

#[tauri::command]
pub fn resume_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.mgr.resume(&id)?)
}

#[tauri::command]
pub fn retry_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.mgr.retry(&id)?)
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    Ok(state.mgr.cancel(&id).await?)
}

#[tauri::command]
pub async fn remove_download(state: State<'_, AppState>, id: String, delete_files: bool) -> CmdResult<()> {
    Ok(state.mgr.remove(&id, delete_files).await?)
}

#[tauri::command]
pub fn pause_all(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.mgr.pause_all()?)
}

#[tauri::command]
pub fn resume_all(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.mgr.resume_all()?)
}

#[tauri::command]
pub fn reorder_queue(state: State<'_, AppState>, ids: Vec<String>) -> CmdResult<()> {
    Ok(state.mgr.reorder(&ids)?)
}

#[tauri::command]
pub fn set_priority(state: State<'_, AppState>, id: String, priority: Priority) -> CmdResult<()> {
    Ok(state.mgr.set_priority(&id, priority)?)
}

#[tauri::command]
pub fn set_category(state: State<'_, AppState>, id: String, category: String) -> CmdResult<()> {
    Ok(state.mgr.set_category(&id, &category)?)
}

#[tauri::command]
pub fn set_download_limit(state: State<'_, AppState>, id: String, limit: Option<u64>) -> CmdResult<()> {
    Ok(state.mgr.set_download_limit(&id, limit)?)
}

#[tauri::command]
pub fn schedule_download(state: State<'_, AppState>, id: String, at: Option<String>) -> CmdResult<()> {
    Ok(state.mgr.schedule(&id, at)?)
}

#[tauri::command]
pub fn set_global_limit(state: State<'_, AppState>, limit: Option<u64>) -> CmdResult<()> {
    Ok(state.mgr.set_global_limit(limit)?)
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.mgr.clear_history()?)
}

#[tauri::command]
pub fn verify_files(state: State<'_, AppState>) -> CmdResult<()> {
    Ok(state.mgr.verify_completed()?)
}

fn completed_path(state: &State<'_, AppState>, id: &str) -> CmdResult<(Download, PathBuf)> {
    let d = state.mgr.get(id)?.ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "Download not found"))?;
    let p = d.full_path();
    Ok((d, p))
}

/// Opens a completed file with its default application (explicit user action only).
#[tauri::command]
pub fn open_file(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let (d, p) = completed_path(&state, &id)?;
    if d.status != DownloadStatus::Completed {
        return Err(CommandError::msg(ErrorKind::Unknown, "The download is not complete yet"));
    }
    if !p.exists() {
        let _ = state.mgr.verify_completed();
        return Err(CommandError::msg(ErrorKind::Filesystem, "The file was moved or deleted"));
    }
    app.opener().open_path(p.to_string_lossy(), None::<&str>).map_err(|e| CommandError { kind: ErrorKind::Filesystem, message: "Could not open the file".into(), detail: Some(e.to_string()) })
}

/// Shows the file in Explorer (or opens its folder).
#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let (d, p) = completed_path(&state, &id)?;
    let result = if p.exists() && d.status == DownloadStatus::Completed {
        app.opener().reveal_item_in_dir(&p)
    } else {
        let dir = Path::new(&d.save_dir);
        let _ = std::fs::create_dir_all(dir);
        app.opener().open_path(dir.to_string_lossy(), None::<&str>)
    };
    result.map_err(|e| CommandError { kind: ErrorKind::Filesystem, message: "Could not open the folder".into(), detail: Some(e.to_string()) })
}

#[tauri::command]
pub fn open_downloads_folder(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let dir = state.mgr.settings().default_download_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    app.opener().open_path(dir, None::<&str>).map_err(|e| CommandError { kind: ErrorKind::Filesystem, message: "Could not open the folder".into(), detail: Some(e.to_string()) })
}

#[tauri::command]
pub fn open_logs_folder(app: AppHandle) -> CmdResult<()> {
    let dir = pixidl_core::paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| CommandError { kind: ErrorKind::Filesystem, message: "Could not open the folder".into(), detail: Some(e.to_string()) })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    (*state.mgr.settings()).clone()
}

/// Saves settings and applies them. Returns human-readable problems with values
/// that had to be corrected.
#[tauri::command]
pub async fn update_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<Vec<String>> {
    let before = state.mgr.settings();
    let problems = state.mgr.update_settings(settings).await?;
    let after = state.mgr.settings();
    if before.launch_at_startup != after.launch_at_startup {
        let al = app.autolaunch();
        let r = if after.launch_at_startup { al.enable() } else { al.disable() };
        if let Err(e) = r {
            tracing::warn!(error = %e, "autostart change failed");
        }
    }
    if before.browser_integration != after.browser_integration || before.allowed_extension_ids != after.allowed_extension_ids || before.allowed_firefox_ids != after.allowed_firefox_ids {
        crate::apply_browser_integration(&app).await;
    }
    Ok(problems)
}

#[tauri::command]
pub async fn complete_first_run(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<Vec<String>> {
    let mut s = settings;
    s.first_run_completed = true;
    std::fs::create_dir_all(&s.default_download_dir).map_err(|e| CommandError::from(pixidl_core::DownloadError::fs("Cannot create the download folder", &e)))?;
    update_settings(app, state, s).await
}

#[tauri::command]
pub fn get_categories(state: State<'_, AppState>) -> CmdResult<Vec<Category>> {
    Ok(state.mgr.categories()?)
}

#[tauri::command]
pub fn upsert_category(state: State<'_, AppState>, category: Category) -> CmdResult<()> {
    Ok(state.mgr.upsert_category(&category)?)
}

#[tauri::command]
pub fn delete_category(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    Ok(state.mgr.delete_category(&name)?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub js_runtime: ToolStatus,
    pub ytdlp: ToolStatus,
    pub ffmpeg: ToolStatus,
    pub torrent: TorrentEngineStatus,
    pub http: ToolStatus,
}

#[tauri::command]
pub async fn get_engine_status(state: State<'_, AppState>) -> CmdResult<EngineStatus> {
    let (ytdlp, ffmpeg, torrent) = state.mgr.engine_status().await;
    Ok(EngineStatus {
        js_runtime: state.mgr.js_runtime_status().await,
        ytdlp,
        ffmpeg,
        torrent,
        http: ToolStatus { name: "http".into(), available: true, path: None, version: Some("built-in".into()), message: None },
    })
}

#[tauri::command]
pub async fn install_ytdlp(state: State<'_, AppState>) -> CmdResult<String> {
    let p = state.mgr.install_ytdlp(&pixidl_core::paths::engines_dir()).await?;
    Ok(p.display().to_string())
}

#[tauri::command]
pub async fn install_deno(state: State<'_, AppState>) -> CmdResult<String> {
    let p = state.mgr.install_deno(&pixidl_core::paths::engines_dir()).await?;
    Ok(p.display().to_string())
}

/// Name/size/type of several links (multi-link paste in the Add dialog).
#[tauri::command]
pub async fn probe_links(state: State<'_, AppState>, urls: Vec<String>) -> CmdResult<Vec<LinkProbe>> {
    if urls.len() > pixidl_core::protocol::MAX_BATCH {
        return Err(CommandError::msg(ErrorKind::Unknown, "Too many links at once (max 200)"));
    }
    Ok(state.mgr.probe_links(urls.into_iter().map(|u| (u, None)).collect()).await)
}

/// Updates yt-dlp now: the copy pixidl manages is replaced with the latest
/// verified release; a user-installed yt-dlp updates itself (`-U`).
#[tauri::command]
pub async fn update_ytdlp(state: State<'_, AppState>) -> CmdResult<String> {
    let s = state.mgr.settings();
    if s.ytdlp_path.trim().is_empty() {
        if let Some(p) = state.mgr.auto_update_ytdlp(true).await? {
            return Ok(format!("Installed the latest yt-dlp: {}", p.display()));
        }
        if state.mgr.engine_status().await.0.available {
            return Ok("yt-dlp is up to date".into());
        }
    }
    Ok(state.mgr.update_ytdlp().await?)
}

#[tauri::command]
pub async fn install_ffmpeg(state: State<'_, AppState>) -> CmdResult<String> {
    let p = state.mgr.install_ffmpeg(&pixidl_core::paths::engines_dir().join("ffmpeg")).await?;
    Ok(p.display().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserIntegrationStatus {
    pub enabled: bool,
    pub bridge_running: bool,
    pub host_path: Option<String>,
    pub host_installed: bool,
    pub browsers: Vec<BrowserRegistration>,
    pub reference_extension_id: String,
    pub protocol_version: u32,
}

#[tauri::command]
pub fn get_browser_integration(state: State<'_, AppState>) -> BrowserIntegrationStatus {
    let host = browser::default_host_path();
    let installed = host.as_ref().is_some_and(|h| h.exists());
    BrowserIntegrationStatus {
        enabled: state.mgr.settings().browser_integration,
        bridge_running: state.bridge.lock().is_some(),
        host_path: host.as_ref().map(|h| h.display().to_string()),
        host_installed: installed,
        browsers: host.as_ref().filter(|_| installed).map(|h| browser::status(h)).unwrap_or_default(),
        reference_extension_id: browser::REFERENCE_EXTENSION_ID.into(),
        protocol_version: pixidl_core::protocol::PROTOCOL_VERSION,
    }
}

#[tauri::command]
pub async fn reinstall_browser_integration(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Vec<BrowserRegistration>> {
    let host = browser::default_host_path().filter(|h| h.exists()).ok_or_else(|| {
        CommandError::msg(ErrorKind::EngineUnavailable, "The native messaging host is not installed next to the application")
    })?;
    let s = state.mgr.settings();
    let res = browser::register(&host, &s.allowed_extension_ids, &s.allowed_firefox_ids);
    crate::apply_browser_integration(&app).await;
    Ok(res)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
    pub data_dir: String,
    pub logs_dir: String,
    pub platform: String,
    pub arch: String,
    pub protocol_version: u32,
    pub tauri_version: String,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        name: pixidl_core::APP_NAME.into(),
        version: app.package_info().version.to_string(),
        identifier: app.config().identifier.clone(),
        data_dir: pixidl_core::paths::data_dir().display().to_string(),
        logs_dir: pixidl_core::paths::logs_dir().display().to_string(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        protocol_version: pixidl_core::protocol::PROTOCOL_VERSION,
        tauri_version: tauri::VERSION.into(),
    }
}

/// Third-party notices bundled with the app.
#[tauri::command]
pub fn get_licenses(app: AppHandle) -> String {
    let candidates = [
        app.path().resource_dir().ok().map(|d| d.join("THIRD-PARTY-NOTICES.md")),
        Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/THIRD-PARTY-NOTICES.md"))),
    ];
    candidates.into_iter().flatten().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| "License information is not available.".into())
}

#[tauri::command]
pub fn cancel_power_action(app: AppHandle) -> bool {
    crate::power::cancel(&app)
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::quit(app);
}

// ---------------------------------------------------------------- browser extensions

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionBrowser {
    pub browser: browser::Browser,
    pub label: String,
    pub installed: bool,
    pub executable: Option<String>,
    pub package: String,
    pub extensions_page: String,
    /// Last time this browser's extension talked to pixidl.
    pub client: Option<ExtensionClient>,
}

#[tauri::command]
pub fn get_extension_browsers(state: State<'_, AppState>) -> Vec<ExtensionBrowser> {
    let clients = state.mgr.extension_clients();
    browser::EXTENSION_BROWSERS
        .iter()
        .map(|&b| {
            let exe = browser::find_browser_executable(b);
            let key = format!("{b:?}").to_ascii_lowercase();
            ExtensionBrowser {
                browser: b,
                label: b.label().into(),
                installed: exe.is_some(),
                executable: exe.map(|p| p.display().to_string()),
                package: b.package().into(),
                extensions_page: b.extensions_page().into(),
                client: clients.iter().find(|c| c.browser == key).cloned(),
            }
        })
        .collect()
}

/// Folder holding the packaged extension: bundled resources in an installed
/// app, `browser-extension/dist` in development.
fn extension_source(app: &AppHandle) -> Option<PathBuf> {
    let bundled = app.path().resource_dir().ok().map(|d| d.join("extension"));
    let dev = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../browser-extension/dist"));
    [bundled, Some(dev)].into_iter().flatten().find(|p| p.join("chromium").join("manifest.json").is_file())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedExtension {
    /// Unpacked folder to load ("Load unpacked" / Firefox temporary add-on).
    pub folder: String,
    /// The package saved into the downloads folder (.zip / .xpi).
    pub archive: String,
    pub extensions_page: String,
}

/// "Download" the extension for a browser: saves the package into the
/// downloads folder and keeps an unpacked copy in a stable location.
#[tauri::command]
pub fn get_extension(app: AppHandle, state: State<'_, AppState>, browser_name: String) -> CmdResult<PreparedExtension> {
    let b = browser::Browser::parse(&browser_name).ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "Unknown browser"))?;
    let src = extension_source(&app).ok_or_else(|| CommandError::msg(ErrorKind::EngineUnavailable, "The extension package is missing from this build"))?;
    let pkg = b.package();
    let io = |e: std::io::Error| CommandError::from(pixidl_core::DownloadError::fs("Could not prepare the extension", &e));
    // Stable unpacked copy (the browser keeps loading from this folder).
    let folder = pixidl_core::paths::data_dir().join("extension").join(pkg);
    if folder.exists() {
        std::fs::remove_dir_all(&folder).map_err(io)?;
    }
    browser::copy_dir(&src.join(pkg), &folder).map_err(io)?;
    // Package file in the user's downloads folder.
    let archive_name = if b.is_firefox() { "pixidl-firefox.xpi" } else { "pixidl-chromium.zip" };
    let dir = PathBuf::from(&state.mgr.settings().default_download_dir);
    std::fs::create_dir_all(&dir).map_err(io)?;
    let label = format!("{b:?}").to_ascii_lowercase();
    let target = dir.join(if b.is_firefox() { "pixidl-extension-firefox.xpi".to_string() } else { format!("pixidl-extension-{label}.zip") });
    std::fs::copy(src.join(archive_name), &target).map_err(io)?;
    tracing::info!(browser = %label, "extension package prepared");
    Ok(PreparedExtension { folder: folder.display().to_string(), archive: target.display().to_string(), extensions_page: b.extensions_page().into() })
}

/// Opens the browser's extensions page (chrome://extensions etc.). These
/// pages can only be opened by starting the browser with the URL.
#[tauri::command]
pub fn open_browser_extensions_page(browser_name: String) -> CmdResult<()> {
    let b = browser::Browser::parse(&browser_name).ok_or_else(|| CommandError::msg(ErrorKind::Unknown, "Unknown browser"))?;
    let exe = browser::find_browser_executable(b).ok_or_else(|| CommandError::msg(ErrorKind::EngineUnavailable, format!("{} was not found on this computer", b.label())))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(b.extensions_page()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| CommandError { kind: ErrorKind::EngineUnavailable, message: format!("Could not start {}", b.label()), detail: Some(e.to_string()) })
}

/// Shows a file or folder created by pixidl in the file manager.
#[tauri::command]
pub fn reveal_path(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    let allowed = [pixidl_core::paths::data_dir(), PathBuf::from(&state.mgr.settings().default_download_dir)];
    if !allowed.iter().any(|a| p.starts_with(a)) {
        return Err(CommandError::msg(ErrorKind::PermissionDenied, "Path is outside pixidl's folders"));
    }
    let r = if p.is_dir() { app.opener().open_path(p.to_string_lossy(), None::<&str>) } else { app.opener().reveal_item_in_dir(&p) };
    r.map_err(|e| CommandError { kind: ErrorKind::Filesystem, message: "Could not open the folder".into(), detail: Some(e.to_string()) })
}

/// Translated tray menu and notification texts from the UI.
#[tauri::command]
pub fn set_native_labels(labels: crate::labels::NativeLabels) {
    crate::labels::set(labels);
    crate::tray::relabel();
}
