//! pixidl — desktop shell (Tauri 2).
//!
//! Wires the Tauri-independent `pixidl-core` into the desktop: IPC commands,
//! UI events, tray, notifications, single-instance handling, autostart,
//! clipboard monitoring, the browser bridge and graceful shutdown.

mod clipboard;
mod commands;
mod error;
mod labels;
mod logging;
mod power;
mod state;
mod tray;
mod updates;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use base64::Engine as _;
use pixidl_core::bridge::BridgeServer;
use pixidl_core::manager::{AddSource, DownloadManager, ManagerConfig};
use pixidl_core::settings::CloseBehavior;
use pixidl_core::types::{AddDownloadRequest, ManagerEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;

pub const EVENT_CHANNEL: &str = "pixidl://event";

fn window_is_active(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if window_is_active(app) {
        return; // the in-app toast is enough
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!(error = %e, "notification failed");
    }
}

/// Receives manager events: forwards them to the UI and raises desktop
/// notifications according to the user's settings.
fn on_manager_event(app: &AppHandle, event: ManagerEvent) {
    let _ = app.emit(EVENT_CHANNEL, &event);
    let Some(state) = app.try_state::<AppState>() else { return };
    let s = state.mgr.settings();
    match &event {
        ManagerEvent::DownloadCompleted { download } if s.notify_completed => {
            let l = labels::get();
            let title = if download.engine == pixidl_core::types::EngineKind::Torrent { &l.torrent_completed } else { &l.download_completed };
            notify(app, title, &download.filename);
        }
        ManagerEvent::DownloadFailed { download } if s.notify_failed => {
            notify(app, &labels::get().download_failed, &format!("{} — {}", download.filename, download.error_message.clone().unwrap_or_default()));
        }
        ManagerEvent::ShowAddDialog { .. } => tray::show_main(app),
        ManagerEvent::QueueFinished => {
            if s.notify_queue_finished {
                let l = labels::get();
                notify(app, &l.queue_finished, &l.queue_finished_body);
            }
            if s.schedule.enabled {
                power::schedule(app, s.schedule.after_queue);
            }
        }
        _ => {}
    }
}

/// Starts or stops the loopback bridge and (re)registers the native host.
pub async fn apply_browser_integration(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.mgr.settings();
    if !s.browser_integration {
        if let Some(b) = state.bridge.lock().take() {
            b.stop();
        }
        return;
    }
    if state.bridge.lock().is_none() {
        match BridgeServer::start(state.mgr.clone(), &pixidl_core::paths::bridge_file()).await {
            Ok(b) => *state.bridge.lock() = Some(b),
            Err(e) => tracing::error!(error = %e, "could not start browser bridge"),
        }
    }
    // Keep the host registration pointing at this installation.
    if let Some(host) = pixidl_core::browser::default_host_path().filter(|h| h.exists()) {
        let res = pixidl_core::browser::register(&host, &s.allowed_extension_ids, &s.allowed_firefox_ids);
        let failed: Vec<_> = res.iter().filter(|r| !r.registered).map(|r| r.label.clone()).collect();
        if !failed.is_empty() {
            tracing::warn!(?failed, "native host registration failed for some browsers");
        }
    } else {
        tracing::info!("native messaging host not found next to the app; browser integration unavailable");
    }
}

/// Handles magnet links and .torrent files passed on the command line
/// (file association / second instance).
fn handle_args(app: &AppHandle, args: &[String]) {
    let mut requests = Vec::new();
    for a in args.iter().skip(1) {
        if a.starts_with("magnet:?") {
            requests.push(AddDownloadRequest { url: a.clone(), ..Default::default() });
        } else if a.to_ascii_lowercase().ends_with(".torrent") {
            let p = PathBuf::from(a);
            match std::fs::metadata(&p) {
                Ok(m) if m.is_file() && m.len() <= 16 * 1024 * 1024 => {
                    if let Ok(bytes) = std::fs::read(&p) {
                        requests.push(AddDownloadRequest { torrent_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)), ..Default::default() });
                    }
                }
                _ => tracing::warn!("ignored invalid torrent argument"),
            }
        }
    }
    if requests.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        for r in requests {
            if let Err(e) = state.mgr.add(r, AddSource::User).await {
                let _ = app.emit("pixidl://error", serde_json::json!({ "message": e.message, "detail": e.detail }));
            }
        }
        tray::show_main(&app);
    });
}

/// Graceful exit: pause transfers (they resume next launch), stop the bridge.
pub fn quit(app: AppHandle) {
    let state = app.state::<AppState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if let Some(b) = state.bridge.lock().take() {
            b.stop();
        }
        state.mgr.shutdown().await;
        tracing::info!("shutdown complete");
        app.exit(0);
    });
}

pub fn run() {
    let data_dir = pixidl_core::paths::data_dir();
    let log_guard = logging::init(&pixidl_core::paths::logs_dir());
    std::panic::set_hook(Box::new(|info| {
        tracing::error!(panic = %info, "panic");
    }));
    tracing::info!(version = pixidl_core::APP_VERSION, "starting pixidl");
    let background = std::env::args().any(|a| a == "--background");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            handle_args(app, &args);
            if !args.iter().any(|a| a == "--background") {
                tray::show_main(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .setup(move |app| {
            let handle = app.handle().clone();
            let mut tool_dirs = vec![pixidl_core::paths::engines_dir()];
            if let Ok(res) = app.path().resource_dir() {
                tool_dirs.insert(0, res.join("bin"));
            }
            let sink_handle = handle.clone();
            let mgr = tauri::async_runtime::block_on(DownloadManager::start(
                ManagerConfig { data_dir: data_dir.clone(), tool_dirs },
                Arc::new(move |e: ManagerEvent| on_manager_event(&sink_handle, e)),
            ))
            .map_err(|e| format!("Could not open the download database: {} {}", e.message, e.detail.unwrap_or_default()))?;
            let settings = mgr.settings();
            app.manage(AppState { mgr, bridge: Default::default(), power_task: Default::default(), quitting: Default::default() });

            tray::setup(&handle)?;
            clipboard::spawn(handle.clone());
            let h = handle.clone();
            tauri::async_runtime::spawn(async move { apply_browser_integration(&h).await });
            // YouTube changes often; keep yt-dlp current in the background (daily).
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                if let Err(e) = h.state::<AppState>().mgr.auto_update_ytdlp(false).await {
                    tracing::warn!(error = %e, "yt-dlp update check failed");
                }
            });
            // App updates from GitHub releases.
            app.manage(updates::UpdaterState::default());
            updates::spawn_background_checks(handle.clone());

            if let Some(w) = app.get_webview_window("main") {
                fit_to_screen(&w);
                if !(background || (settings.start_minimized && settings.first_run_completed)) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            handle_args(&handle, &std::env::args().collect::<Vec<_>>());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let state = app.state::<AppState>();
                if state.quitting.load(Ordering::SeqCst) {
                    return;
                }
                let s = state.mgr.settings();
                if s.close_behavior == CloseBehavior::MinimizeToTray && s.minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    api.prevent_close();
                    quit(app.clone());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_downloads,
            commands::get_download,
            commands::get_download_events,
            commands::get_stats,
            commands::add_download,
            commands::inspect_url,
            commands::read_torrent_file,
            commands::pause_download,
            commands::resume_download,
            commands::retry_download,
            commands::cancel_download,
            commands::remove_download,
            commands::pause_all,
            commands::resume_all,
            commands::reorder_queue,
            commands::set_priority,
            commands::set_category,
            commands::set_download_limit,
            commands::schedule_download,
            commands::set_global_limit,
            commands::clear_history,
            commands::clear_completed_downloads,
            commands::resume_downloads,
            commands::pause_downloads,
            commands::remove_downloads,
            commands::move_to_queue,
            commands::list_queues,
            commands::create_queue,
            commands::rename_queue,
            commands::set_queue_max_concurrent,
            commands::delete_queue,
            commands::start_queue,
            commands::stop_queue,
            commands::verify_files,
            commands::open_file,
            commands::open_folder,
            commands::open_downloads_folder,
            commands::open_logs_folder,
            commands::get_settings,
            commands::update_settings,
            commands::complete_first_run,
            commands::get_categories,
            commands::upsert_category,
            commands::delete_category,
            commands::get_engine_status,
            commands::install_ytdlp,
            commands::update_ytdlp,
            commands::get_browser_integration,
            commands::reinstall_browser_integration,
            commands::get_app_info,
            commands::get_licenses,
            commands::cancel_power_action,
            commands::quit_app,
            commands::get_extension_browsers,
            commands::get_extension,
            commands::open_browser_extensions_page,
            commands::reveal_path,
            commands::install_deno,
            commands::install_ffmpeg,
            commands::set_native_labels,
            commands::probe_links,
            // app updates
            commands::get_update_status,
            commands::check_for_updates,
            commands::download_update,
            commands::cancel_update_download,
            commands::install_update,
            commands::open_project_page,
        ])
        .build(tauri::generate_context!())
        .expect("error while building pixidl")
        .run(move |app, event| {
            let _keep = &log_guard;
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // Exit requested by the OS/runtime (not our own quit): shut down gracefully.
                let state = app.state::<AppState>();
                if code.is_none() && !state.quitting.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit(app.clone());
                }
            }
        });
}

/// The configured window size (1200×780) is taller than many laptop screens
/// once Windows display scaling applies; shrink it to the monitor's work area so
/// the sidebar footer and status bar are never off-screen.
fn fit_to_screen(w: &tauri::WebviewWindow) {
    let (Ok(Some(monitor)), Ok(size)) = (w.current_monitor(), w.outer_size()) else { return };
    let area = monitor.work_area().size;
    let max_w = (area.width as f64 * 0.95) as u32;
    let max_h = (area.height as f64 * 0.92) as u32;
    if size.width > max_w || size.height > max_h {
        let _ = w.set_size(tauri::PhysicalSize::new(size.width.min(max_w), size.height.min(max_h)));
        let _ = w.center();
    }
}
