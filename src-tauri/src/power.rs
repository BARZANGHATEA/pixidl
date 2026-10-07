//! Optional, explicit "after the queue finishes" action (sleep / shut down).
//! Always preceded by a 60-second countdown the user can cancel from the UI
//! or the notification. Commands are run with argument arrays, never a shell.

use std::process::Command;
use std::time::Duration;

use nexa_core::settings::AfterQueueAction;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

pub const COUNTDOWN_SECS: u64 = 60;

pub fn schedule(app: &AppHandle, action: AfterQueueAction) {
    if action == AfterQueueAction::Nothing {
        return;
    }
    let state = app.state::<AppState>();
    let mut slot = state.power_task.lock();
    if slot.is_some() {
        return;
    }
    let _ = app.emit("nexa://power-countdown", serde_json::json!({ "action": action, "seconds": COUNTDOWN_SECS }));
    let handle = app.clone();
    *slot = Some(tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(COUNTDOWN_SECS)).await;
        tracing::info!(?action, "running after-queue power action");
        handle.state::<AppState>().power_task.lock().take();
        if let Err(e) = run(action) {
            tracing::error!(error = %e, "power action failed");
        }
    }));
}

pub fn cancel(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let task = state.power_task.lock().take();
    match task {
        Some(t) => {
            t.abort();
            let _ = app.emit("nexa://power-cancelled", ());
            true
        }
        None => false,
    }
}

fn run(action: AfterQueueAction) -> std::io::Result<()> {
    let (program, args): (&str, &[&str]) = match action {
        AfterQueueAction::Nothing => return Ok(()),
        #[cfg(windows)]
        AfterQueueAction::Shutdown => ("shutdown.exe", &["/s", "/t", "0"]),
        #[cfg(windows)]
        AfterQueueAction::Sleep => ("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]),
        #[cfg(target_os = "macos")]
        AfterQueueAction::Shutdown => ("osascript", &["-e", "tell app \"System Events\" to shut down"]),
        #[cfg(target_os = "macos")]
        AfterQueueAction::Sleep => ("pmset", &["sleepnow"]),
        #[cfg(all(unix, not(target_os = "macos")))]
        AfterQueueAction::Shutdown => ("systemctl", &["poweroff"]),
        #[cfg(all(unix, not(target_os = "macos")))]
        AfterQueueAction::Sleep => ("systemctl", &["suspend"]),
    };
    Command::new(program).args(args).spawn().map(|_| ())
}
