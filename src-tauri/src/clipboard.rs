//! Optional clipboard monitoring. Clipboard text is only inspected locally,
//! never logged, stored or sent anywhere. Only a detected URL is passed to
//! the UI, which offers to add it.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::state::AppState;

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<u64> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let enabled = app.state::<AppState>().mgr.settings().clipboard_monitoring;
            if !enabled {
                last = None;
                continue;
            }
            let Ok(text) = app.clipboard().read_text() else { continue };
            let text = text.trim();
            if text.is_empty() || text.len() > 8192 {
                continue;
            }
            // Compare by hash so the previous clipboard content is not kept around.
            let h = {
                use std::hash::{Hash, Hasher};
                let mut s = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut s);
                s.finish()
            };
            if last == Some(h) {
                continue;
            }
            let first = last.is_none();
            last = Some(h);
            if first {
                continue; // don't react to whatever was already copied at startup
            }
            if let Ok((url, engine)) = pixidl_core::detector::detect(text) {
                if url.scheme() != "ftp" {
                    let _ = app.emit("pixidl://clipboard-url", serde_json::json!({ "url": url.as_str(), "engine": engine }));
                }
            }
        }
    });
}
