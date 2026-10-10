//! Texts the Rust side shows itself (tray menu, desktop notifications). The UI
//! sends them translated whenever the language changes; English until then.

use std::sync::{LazyLock, RwLock};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NativeLabels {
    pub show: String,
    pub pause_all: String,
    pub resume_all: String,
    pub open_dir: String,
    pub settings: String,
    pub exit: String,
    /// "{{n}}" is replaced.
    pub active: String,
    /// "{{speed}}" is replaced.
    pub speed: String,
    pub download_completed: String,
    pub torrent_completed: String,
    pub download_failed: String,
    pub queue_finished: String,
    pub queue_finished_body: String,
}

impl Default for NativeLabels {
    fn default() -> Self {
        Self {
            show: "Show".into(),
            pause_all: "Pause All".into(),
            resume_all: "Resume All".into(),
            open_dir: "Open Downloads Folder".into(),
            settings: "Settings".into(),
            exit: "Exit".into(),
            active: "Active: {{n}}".into(),
            speed: "Speed: {{speed}}".into(),
            download_completed: "Download completed".into(),
            torrent_completed: "Torrent completed".into(),
            download_failed: "Download failed".into(),
            queue_finished: "All downloads finished".into(),
            queue_finished_body: "The download queue is empty.".into(),
        }
    }
}

static LABELS: LazyLock<RwLock<NativeLabels>> = LazyLock::new(Default::default);

pub fn get() -> NativeLabels {
    LABELS.read().map(|l| l.clone()).unwrap_or_default()
}

/// Stores new labels; empty or overlong values keep the English default.
pub fn set(mut new: NativeLabels) {
    let d = NativeLabels::default();
    for (v, fallback) in [
        (&mut new.show, d.show),
        (&mut new.pause_all, d.pause_all),
        (&mut new.resume_all, d.resume_all),
        (&mut new.open_dir, d.open_dir),
        (&mut new.settings, d.settings),
        (&mut new.exit, d.exit),
        (&mut new.active, d.active),
        (&mut new.speed, d.speed),
        (&mut new.download_completed, d.download_completed),
        (&mut new.torrent_completed, d.torrent_completed),
        (&mut new.download_failed, d.download_failed),
        (&mut new.queue_finished, d.queue_finished),
        (&mut new.queue_finished_body, d.queue_finished_body),
    ] {
        if v.trim().is_empty() || v.chars().count() > 120 {
            *v = fallback;
        }
    }
    if let Ok(mut l) = LABELS.write() {
        *l = new;
    }
}
