//! System tray: live status plus quick actions.

use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::labels;
use crate::state::AppState;

/// Menu items whose text follows the UI language.
static ITEMS: std::sync::OnceLock<Vec<(&'static str, MenuItem<Wry>)>> = std::sync::OnceLock::new();

/// Applies the current labels to the tray menu.
pub fn relabel() {
    let l = labels::get();
    for (id, item) in ITEMS.get().into_iter().flatten() {
        let text = match *id {
            "show" => &l.show,
            "pause_all" => &l.pause_all,
            "resume_all" => &l.resume_all,
            "open_dir" => &l.open_dir,
            "settings" => &l.settings,
            "exit" => &l.exit,
            _ => continue,
        };
        let _ = item.set_text(text);
    }
}

pub fn format_speed(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut v = bps as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bps} B/s")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let title = MenuItem::with_id(app, "title", "pixidl", false, None::<&str>)?;
    let active = MenuItem::with_id(app, "active", "Active: 0", false, None::<&str>)?;
    let speed = MenuItem::with_id(app, "speed", "Speed: 0 B/s", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let pause_all = MenuItem::with_id(app, "pause_all", "Pause All", true, None::<&str>)?;
    let resume_all = MenuItem::with_id(app, "resume_all", "Resume All", true, None::<&str>)?;
    let open_dir = MenuItem::with_id(app, "open_dir", "Open Downloads Folder", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let _ = ITEMS.set(vec![
        ("show", show.clone()),
        ("pause_all", pause_all.clone()),
        ("resume_all", resume_all.clone()),
        ("open_dir", open_dir.clone()),
        ("settings", settings.clone()),
        ("exit", exit.clone()),
    ]);
    let menu: Menu<Wry> = Menu::with_items(app, &[&title, &sep()?, &active, &speed, &sep()?, &show, &pause_all, &resume_all, &open_dir, &settings, &sep()?, &exit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("pixidl")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>();
            match event.id().as_ref() {
                "show" => show_main(app),
                "pause_all" => {
                    let _ = state.mgr.pause_all();
                }
                "resume_all" => {
                    let _ = state.mgr.resume_all();
                }
                "open_dir" => {
                    let dir = state.mgr.settings().default_download_dir.clone();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = app.opener().open_path(dir, None::<&str>);
                }
                "settings" => {
                    show_main(app);
                    let _ = app.emit("pixidl://navigate", "settings");
                }
                "exit" => crate::quit(app.clone()),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    // Refresh the live status lines.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Ok(stats) = handle.state::<AppState>().mgr.stats() else { continue };
            let l = labels::get();
            let active_text = l.active.replace("{{n}}", &stats.active.to_string());
            let speed_text = l.speed.replace("{{speed}}", &format_speed(stats.download_bps));
            let _ = active.set_text(&active_text);
            let _ = speed.set_text(&speed_text);
            if let Some(tray) = handle.tray_by_id("main") {
                let _ = tray.set_tooltip(Some(format!("pixidl — {active_text} · {speed_text}")));
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn speed_format() {
        assert_eq!(super::format_speed(0), "0 B/s");
        assert_eq!(super::format_speed(20_447_232), "19.5 MB/s");
        assert_eq!(super::format_speed(1536), "1.5 KB/s");
    }
}
