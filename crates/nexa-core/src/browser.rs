//! Native-messaging host registration for Chromium browsers and Firefox.
//!
//! Generates the host manifests and registers them (HKCU registry keys on
//! Windows, per-user manifest folders on Linux/macOS). Used by the installer
//! (`nexa-native-host --register`), by the app at startup and by
//! Settings → Browser integration → Reinstall.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::paths::{native_messaging_dir, NATIVE_HOST_NAME};

/// ID of the bundled reference extension (derived from the public key in
/// `browser-extension/manifest.json`).
pub const REFERENCE_EXTENSION_ID: &str = "ndlafmjbcbcjmkegfelbhgknmajgdbna";
/// Gecko ID of the reference extension for Firefox.
pub const REFERENCE_FIREFOX_ID: &str = "nexa@nexa-download-manager.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Chromium,
    Vivaldi,
    Firefox,
}

impl Browser {
    pub const ALL: [Browser; 6] = [Browser::Chrome, Browser::Edge, Browser::Brave, Browser::Chromium, Browser::Vivaldi, Browser::Firefox];

    pub fn label(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Edge => "Microsoft Edge",
            Browser::Brave => "Brave",
            Browser::Chromium => "Chromium",
            Browser::Vivaldi => "Vivaldi",
            Browser::Firefox => "Mozilla Firefox",
        }
    }

    pub fn is_firefox(self) -> bool {
        self == Browser::Firefox
    }

    #[cfg(windows)]
    fn registry_key(self) -> String {
        let base = match self {
            Browser::Chrome | Browser::Vivaldi => r"Software\Google\Chrome",
            Browser::Edge => r"Software\Microsoft\Edge",
            Browser::Brave => r"Software\BraveSoftware\Brave-Browser",
            Browser::Chromium => r"Software\Chromium",
            Browser::Firefox => r"Software\Mozilla",
        };
        format!(r"{base}\NativeMessagingHosts\{NATIVE_HOST_NAME}")
    }

    /// Per-user manifest directory (Linux/macOS).
    #[cfg(not(windows))]
    fn manifest_dir(self) -> Option<PathBuf> {
        let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
        #[cfg(target_os = "macos")]
        let p = {
            let sup = home.join("Library/Application Support");
            match self {
                Browser::Chrome => sup.join("Google/Chrome/NativeMessagingHosts"),
                Browser::Edge => sup.join("Microsoft Edge/NativeMessagingHosts"),
                Browser::Brave => sup.join("BraveSoftware/Brave-Browser/NativeMessagingHosts"),
                Browser::Chromium => sup.join("Chromium/NativeMessagingHosts"),
                Browser::Vivaldi => sup.join("Vivaldi/NativeMessagingHosts"),
                Browser::Firefox => sup.join("Mozilla/NativeMessagingHosts"),
            }
        };
        #[cfg(not(target_os = "macos"))]
        let p = {
            let cfg = home.join(".config");
            match self {
                Browser::Chrome => cfg.join("google-chrome/NativeMessagingHosts"),
                Browser::Edge => cfg.join("microsoft-edge/NativeMessagingHosts"),
                Browser::Brave => cfg.join("BraveSoftware/Brave-Browser/NativeMessagingHosts"),
                Browser::Chromium => cfg.join("chromium/NativeMessagingHosts"),
                Browser::Vivaldi => cfg.join("vivaldi/NativeMessagingHosts"),
                Browser::Firefox => home.join(".mozilla/native-messaging-hosts"),
            }
        };
        Some(p)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserRegistration {
    pub browser: Browser,
    pub label: String,
    pub registered: bool,
    pub manifest_path: Option<String>,
    pub error: Option<String>,
}

/// Chromium manifest (`allowed_origins`).
pub fn chromium_manifest(host_path: &Path, extension_ids: &[String]) -> serde_json::Value {
    let mut ids: Vec<String> = vec![REFERENCE_EXTENSION_ID.to_string()];
    ids.extend(extension_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Nexa Download Manager browser integration",
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": ids.iter().map(|i| format!("chrome-extension://{i}/")).collect::<Vec<_>>(),
    })
}

/// Firefox manifest (`allowed_extensions`).
pub fn firefox_manifest(host_path: &Path, addon_ids: &[String]) -> serde_json::Value {
    let mut ids: Vec<String> = vec![REFERENCE_FIREFOX_ID.to_string()];
    ids.extend(addon_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Nexa Download Manager browser integration",
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": ids,
    })
}

fn write_json(path: &Path, v: &serde_json::Value) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v).unwrap())
}

/// Registers the host for every supported browser. Returns per-browser results.
pub fn register(host_path: &Path, chromium_ids: &[String], firefox_ids: &[String]) -> Vec<BrowserRegistration> {
    let chromium = chromium_manifest(host_path, chromium_ids);
    let firefox = firefox_manifest(host_path, firefox_ids);
    Browser::ALL
        .iter()
        .map(|&b| {
            let manifest = if b.is_firefox() { &firefox } else { &chromium };
            match register_one(b, manifest) {
                Ok(p) => BrowserRegistration { browser: b, label: b.label().into(), registered: true, manifest_path: Some(p.display().to_string()), error: None },
                Err(e) => BrowserRegistration { browser: b, label: b.label().into(), registered: false, manifest_path: None, error: Some(e.to_string()) },
            }
        })
        .collect()
}

#[cfg(windows)]
fn register_one(b: Browser, manifest: &serde_json::Value) -> std::io::Result<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    let file = native_messaging_dir().join(if b.is_firefox() { format!("{NATIVE_HOST_NAME}.firefox.json") } else { format!("{NATIVE_HOST_NAME}.chromium.json") });
    write_json(&file, manifest)?;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(b.registry_key())?;
    key.set_value("", &file.to_string_lossy().to_string())?;
    Ok(file)
}

#[cfg(not(windows))]
fn register_one(b: Browser, manifest: &serde_json::Value) -> std::io::Result<PathBuf> {
    let dir = b.manifest_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    let file = dir.join(format!("{NATIVE_HOST_NAME}.json"));
    write_json(&file, manifest)?;
    // Keep a copy next to the app data for diagnostics.
    let _ = write_json(&native_messaging_dir().join(if b.is_firefox() { "firefox.json" } else { "chromium.json" }), manifest);
    Ok(file)
}

/// Removes all registrations (used by the uninstaller).
pub fn unregister() -> Vec<BrowserRegistration> {
    let out = Browser::ALL
        .iter()
        .map(|&b| {
            let r = unregister_one(b);
            BrowserRegistration { browser: b, label: b.label().into(), registered: false, manifest_path: None, error: r.err().map(|e| e.to_string()) }
        })
        .collect();
    let _ = std::fs::remove_dir_all(native_messaging_dir());
    out
}

#[cfg(windows)]
fn unregister_one(b: Browser) -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.delete_subkey(b.registry_key()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
fn unregister_one(b: Browser) -> std::io::Result<()> {
    if let Some(dir) = b.manifest_dir() {
        match std::fs::remove_file(dir.join(format!("{NATIVE_HOST_NAME}.json"))) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Checks whether each browser points at a manifest for `host_path`.
pub fn status(host_path: &Path) -> Vec<BrowserRegistration> {
    Browser::ALL
        .iter()
        .map(|&b| {
            let found = manifest_path_for(b);
            let ok = found.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok()).map_or(false, |v| {
                v.get("path").and_then(|p| p.as_str()).map_or(false, |p| Path::new(p) == host_path) && host_path.exists()
            });
            BrowserRegistration {
                browser: b,
                label: b.label().into(),
                registered: ok,
                manifest_path: found.map(|p| p.display().to_string()),
                error: (!ok).then(|| "Not registered".to_string()),
            }
        })
        .collect()
}

#[cfg(windows)]
fn manifest_path_for(b: Browser) -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(b.registry_key()).ok()?;
    let v: String = key.get_value("").ok()?;
    Some(PathBuf::from(v))
}

#[cfg(not(windows))]
fn manifest_path_for(b: Browser) -> Option<PathBuf> {
    let p = b.manifest_dir()?.join(format!("{NATIVE_HOST_NAME}.json"));
    p.exists().then_some(p)
}

/// Location of the native host executable next to the app executable.
pub fn default_host_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = crate::tools::exe_name("nexa-native-host");
    let p = dir.join(&name);
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_have_required_fields() {
        let m = chromium_manifest(Path::new("/opt/nexa/nexa-native-host"), &["abcdefghijklmnopabcdefghijklmnop".into()]);
        assert_eq!(m["name"], NATIVE_HOST_NAME);
        assert_eq!(m["type"], "stdio");
        let origins = m["allowed_origins"].as_array().unwrap();
        assert!(origins.contains(&serde_json::json!(format!("chrome-extension://{REFERENCE_EXTENSION_ID}/"))));
        assert_eq!(origins.len(), 2);
        let f = firefox_manifest(Path::new("/x"), &[]);
        assert_eq!(f["allowed_extensions"][0], REFERENCE_FIREFOX_ID);
    }

    #[test]
    fn host_name_is_valid() {
        // Chrome: lowercase alphanumerics, '_' and '.', no leading/trailing dot.
        assert!(NATIVE_HOST_NAME.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_'));
    }
}
