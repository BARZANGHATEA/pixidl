//! Application settings. Persisted in the `settings` table as one JSON value per
//! key so new settings can be added without migrations; unknown/invalid stored
//! values fall back to defaults.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::security::DuplicatePolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CloseBehavior {
    MinimizeToTray,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProxyMode {
    /// Direct connection.
    None,
    /// Use the system proxy (environment / OS settings).
    System,
    /// Use the URL in `proxy_url` (http://, https:// or socks5://).
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AfterQueueAction {
    Nothing,
    Sleep,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScheduleSettings {
    pub enabled: bool,
    /// "HH:MM" local time at which the queue may start.
    pub start_time: String,
    /// "HH:MM" local time at which running downloads are paused back to the queue.
    pub stop_time: String,
    /// Days of week the window applies to (0 = Monday … 6 = Sunday).
    pub days: Vec<u8>,
    /// Optional, explicit action after the queue finishes inside the window.
    pub after_queue: AfterQueueAction,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "23:00".into(),
            stop_time: "07:00".into(),
            days: (0..7).collect(),
            after_queue: AfterQueueAction::Nothing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    // General
    pub language: String,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub close_behavior: CloseBehavior,
    pub first_run_completed: bool,
    pub clipboard_monitoring: bool,

    // Downloads
    pub default_download_dir: String,
    /// Additional approved download folders (destinations must be inside one of these
    /// or inside `default_download_dir`).
    pub extra_download_dirs: Vec<String>,
    pub max_concurrent_downloads: u32,
    pub ask_for_destination: bool,
    pub duplicate_policy: DuplicatePolicy,
    pub connections_per_download: u32,
    pub auto_resume_on_startup: bool,
    pub category_subfolders: bool,

    // Connection
    /// Global download limit in bytes/s; `None` = unlimited.
    #[ts(type = "number | null")]
    pub global_speed_limit_bps: Option<u64>,
    #[ts(type = "number | null")]
    pub global_upload_limit_bps: Option<u64>,
    pub proxy_mode: ProxyMode,
    pub proxy_url: String,
    pub connect_timeout_secs: u32,
    pub read_timeout_secs: u32,
    pub retry_count: u32,
    pub retry_delay_secs: u32,

    // Appearance
    pub theme: Theme,
    pub accent_color: String,

    // Notifications
    pub notify_completed: bool,
    pub notify_failed: bool,
    pub notify_queue_finished: bool,

    // Browser integration
    pub browser_integration: bool,
    /// Extra Chromium extension IDs allowed to talk to the native host
    /// (the bundled reference extension is always allowed).
    pub allowed_extension_ids: Vec<String>,
    /// Firefox add-on IDs allowed to talk to the native host.
    pub allowed_firefox_ids: Vec<String>,

    // Engines
    pub ytdlp_path: String,
    pub ffmpeg_path: String,
    /// JavaScript runtime for yt-dlp's YouTube support (Deno/Node/Bun); empty = auto.
    pub js_runtime_path: String,
    pub torrent_listen_port: u16,
    pub torrent_enable_dht: bool,
    /// Keep seeding after a torrent completes.
    pub torrent_seed_after_completion: bool,

    // Scheduler
    pub schedule: ScheduleSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "en".into(),
            launch_at_startup: false,
            start_minimized: false,
            minimize_to_tray: true,
            close_behavior: CloseBehavior::MinimizeToTray,
            first_run_completed: false,
            clipboard_monitoring: false,
            default_download_dir: default_download_dir(),
            extra_download_dirs: Vec::new(),
            max_concurrent_downloads: 3,
            ask_for_destination: false,
            duplicate_policy: DuplicatePolicy::Rename,
            connections_per_download: 4,
            auto_resume_on_startup: true,
            category_subfolders: false,
            global_speed_limit_bps: None,
            global_upload_limit_bps: None,
            proxy_mode: ProxyMode::System,
            proxy_url: String::new(),
            connect_timeout_secs: 20,
            read_timeout_secs: 60,
            retry_count: 3,
            retry_delay_secs: 3,
            theme: Theme::System,
            accent_color: "#2563EB".into(),
            notify_completed: true,
            notify_failed: true,
            notify_queue_finished: true,
            browser_integration: true,
            allowed_extension_ids: Vec::new(),
            allowed_firefox_ids: Vec::new(),
            ytdlp_path: String::new(),
            ffmpeg_path: String::new(),
            js_runtime_path: String::new(),
            torrent_listen_port: 0,
            torrent_enable_dht: true,
            torrent_seed_after_completion: false,
            schedule: ScheduleSettings::default(),
        }
    }
}

pub fn default_download_dir() -> String {
    directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
        .or_else(|| directories::UserDirs::new().map(|u| u.home_dir().join("Downloads")))
        .unwrap_or_else(|| std::env::temp_dir().join("Downloads"))
        .to_string_lossy()
        .to_string()
}

fn valid_hhmm(s: &str) -> bool {
    parse_hhmm(s).is_some()
}

pub fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

fn valid_extension_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| ('a'..='p').contains(&c))
}

impl Settings {
    /// Clamps/normalises values. Returns a list of human-readable problems with
    /// values that had to be rejected outright.
    pub fn validate(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        self.max_concurrent_downloads = self.max_concurrent_downloads.clamp(1, 20);
        self.connections_per_download = self.connections_per_download.clamp(1, 16);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(3, 300);
        self.read_timeout_secs = self.read_timeout_secs.clamp(5, 600);
        self.retry_count = self.retry_count.min(20);
        self.retry_delay_secs = self.retry_delay_secs.clamp(1, 300);
        if matches!(self.global_speed_limit_bps, Some(0)) {
            self.global_speed_limit_bps = None;
        }
        if matches!(self.global_upload_limit_bps, Some(0)) {
            self.global_upload_limit_bps = None;
        }
        if !["en", "fa"].contains(&self.language.as_str()) {
            self.language = "en".into();
        }
        let accent_ok = self.accent_color.len() == 7
            && self.accent_color.starts_with('#')
            && self.accent_color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !accent_ok {
            problems.push("Invalid accent color".into());
            self.accent_color = Settings::default().accent_color;
        }
        if self.default_download_dir.trim().is_empty() || !std::path::Path::new(&self.default_download_dir).is_absolute() {
            problems.push("Default download folder must be an absolute path".into());
            self.default_download_dir = default_download_dir();
        }
        self.extra_download_dirs.retain(|d| std::path::Path::new(d).is_absolute());
        self.extra_download_dirs.dedup();
        if self.proxy_mode == ProxyMode::Manual {
            match url::Url::parse(self.proxy_url.trim()) {
                Ok(u) if ["http", "https", "socks5", "socks5h"].contains(&u.scheme()) => {
                    if !u.username().is_empty() || u.password().is_some() {
                        problems.push("Proxy credentials are not stored; use an unauthenticated proxy".into());
                        let mut u = u.clone();
                        let _ = u.set_username("");
                        let _ = u.set_password(None);
                        self.proxy_url = u.to_string();
                    }
                }
                _ => {
                    problems.push("Invalid proxy URL".into());
                    self.proxy_mode = ProxyMode::System;
                }
            }
        }
        let before = self.allowed_extension_ids.len();
        self.allowed_extension_ids = self.allowed_extension_ids.iter().map(|s| s.trim().to_string()).filter(|s| valid_extension_id(s)).collect();
        if self.allowed_extension_ids.len() != before {
            problems.push("Ignored invalid Chromium extension ID".into());
        }
        self.allowed_firefox_ids = self
            .allowed_firefox_ids
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() < 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || "@.-_{}".contains(c)))
            .collect();
        if !valid_hhmm(&self.schedule.start_time) {
            problems.push("Invalid schedule start time".into());
            self.schedule.start_time = ScheduleSettings::default().start_time;
        }
        if !valid_hhmm(&self.schedule.stop_time) {
            problems.push("Invalid schedule stop time".into());
            self.schedule.stop_time = ScheduleSettings::default().stop_time;
        }
        self.schedule.days.retain(|d| *d < 7);
        self.schedule.days.sort_unstable();
        self.schedule.days.dedup();
        problems
    }

    /// All directories downloads may be written to.
    pub fn approved_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut v = vec![std::path::PathBuf::from(&self.default_download_dir)];
        v.extend(self.extra_download_dirs.iter().map(std::path::PathBuf::from));
        v
    }

    /// Serialises to (key, json) pairs for storage.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        let v = serde_json::to_value(self).expect("settings serialise");
        v.as_object()
            .expect("object")
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect()
    }

    /// Builds settings from stored pairs; missing or invalid keys use defaults.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut base = serde_json::to_value(Settings::default()).expect("serialise");
        let obj = base.as_object_mut().expect("object");
        for (k, v) in pairs {
            if !obj.contains_key(&k) {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&v) {
                let prev = obj.insert(k.clone(), val);
                // Validate field-by-field so one bad value doesn't reset everything.
                if serde_json::from_value::<Settings>(serde_json::Value::Object(obj.clone())).is_err() {
                    if let Some(p) = prev {
                        obj.insert(k, p);
                    }
                }
            }
        }
        let mut s: Settings = serde_json::from_value(base).unwrap_or_default();
        s.validate();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let mut s = Settings::default();
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert_eq!(s.max_concurrent_downloads, 3);
    }

    #[test]
    fn clamps_values() {
        let mut s = Settings { max_concurrent_downloads: 0, connections_per_download: 99, global_speed_limit_bps: Some(0), ..Default::default() };
        s.validate();
        assert_eq!(s.max_concurrent_downloads, 1);
        assert_eq!(s.connections_per_download, 16);
        assert_eq!(s.global_speed_limit_bps, None);
    }

    #[test]
    fn rejects_bad_values() {
        let mut s = Settings { accent_color: "red".into(), ..Default::default() };
        s.schedule.start_time = "25:00".into();
        s.proxy_mode = ProxyMode::Manual;
        s.proxy_url = "socks5://user:pw@127.0.0.1:1080".into();
        s.allowed_extension_ids = vec!["abcdefghijklmnopabcdefghijklmnop".into(), "bad".into()];
        let p = s.validate();
        assert_eq!(s.accent_color, "#2563EB");
        assert_eq!(s.schedule.start_time, "23:00");
        assert!(!s.proxy_url.contains("pw"));
        assert_eq!(s.allowed_extension_ids.len(), 1);
        assert!(p.len() >= 3);
    }

    #[test]
    fn round_trip_pairs_and_tolerates_garbage() {
        let s = Settings { max_concurrent_downloads: 7, language: "fa".into(), ..Default::default() };
        let mut pairs = s.to_pairs();
        pairs.push(("unknownKey".into(), "1".into()));
        pairs.retain(|(k, _)| k != "theme");
        pairs.push(("theme".into(), "\"neon\"".into()));
        pairs.push(("retryCount".into(), "not json".into()));
        let back = Settings::from_pairs(pairs);
        assert_eq!(back.max_concurrent_downloads, 7);
        assert_eq!(back.language, "fa");
        assert_eq!(back.theme, Theme::System);
        assert_eq!(back.retry_count, 3);
    }

    #[test]
    fn hhmm() {
        assert_eq!(parse_hhmm("23:05"), Some((23, 5)));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("7:00"), Some((7, 0)));
    }
}
