//! Native-messaging host registration for Chromium browsers and Firefox.
//!
//! Generates the host manifests and registers them (HKCU registry keys on
//! Windows, per-user manifest folders on Linux/macOS). Used by the installer
//! (`pixidl-native-host --register`), by the app at startup and by
//! Settings → Browser integration → Reinstall.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::paths::{native_messaging_dir, NATIVE_HOST_NAME};

/// ID of the bundled reference extension (derived from the public key in
/// `browser-extension/manifest.json`).
pub const REFERENCE_EXTENSION_ID: &str = "ndlafmjbcbcjmkegfelbhgknmajgdbna";
/// Gecko ID of the reference extension for Firefox.
pub const REFERENCE_FIREFOX_ID: &str = "pixidl@pixidl.app";

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
        "description": "pixidl browser integration",
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
        "description": "pixidl browser integration",
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
            let ok = found.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok()).is_some_and(|v| {
                v.get("path").and_then(|p| p.as_str()).is_some_and(|p| Path::new(p) == host_path) && host_path.exists()
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
    let name = crate::tools::exe_name("pixidl-native-host");
    let p = dir.join(&name);
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_have_required_fields() {
        let m = chromium_manifest(Path::new("/opt/pixidl/pixidl-native-host"), &["abcdefghijklmnopabcdefghijklmnop".into()]);
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

// ---------------------------------------------------------------------------
// Installed browsers (for the in-app Extensions page)

/// Browsers the extension supports, as shown in the app.
pub const EXTENSION_BROWSERS: [Browser; 4] = [Browser::Chrome, Browser::Edge, Browser::Brave, Browser::Firefox];

impl Browser {
    /// Extensions-management page of the browser.
    pub fn extensions_page(self) -> &'static str {
        match self {
            Browser::Chrome | Browser::Chromium => "chrome://extensions/",
            Browser::Edge => "edge://extensions/",
            Browser::Brave => "brave://extensions/",
            Browser::Vivaldi => "vivaldi://extensions/",
            Browser::Firefox => "about:debugging#/runtime/this-firefox",
        }
    }

    /// Which extension package the browser uses.
    pub fn package(self) -> &'static str {
        if self.is_firefox() {
            "firefox"
        } else {
            "chromium"
        }
    }

    pub fn parse(s: &str) -> Option<Browser> {
        Some(match s {
            "chrome" => Browser::Chrome,
            "edge" => Browser::Edge,
            "brave" => Browser::Brave,
            "chromium" => Browser::Chromium,
            "vivaldi" => Browser::Vivaldi,
            "firefox" => Browser::Firefox,
            _ => return None,
        })
    }
}

/// Finds the browser executable (Windows: App Paths registry + standard
/// install folders; Linux: PATH; macOS: /Applications).
pub fn find_browser_executable(b: Browser) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let exe = match b {
            Browser::Chrome => "chrome.exe",
            Browser::Edge => "msedge.exe",
            Browser::Brave => "brave.exe",
            Browser::Chromium => "chromium.exe",
            Browser::Vivaldi => "vivaldi.exe",
            Browser::Firefox => "firefox.exe",
        };
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let key = winreg::RegKey::predef(hive).open_subkey(format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}"));
            if let Ok(k) = key {
                if let Ok(v) = k.get_value::<String, _>("") {
                    let p = PathBuf::from(v.trim_matches('"'));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        let rel = match b {
            Browser::Chrome => r"Google\Chrome\Application\chrome.exe",
            Browser::Edge => r"Microsoft\Edge\Application\msedge.exe",
            Browser::Brave => r"BraveSoftware\Brave-Browser\Application\brave.exe",
            Browser::Chromium => r"Chromium\Application\chrome.exe",
            Browser::Vivaldi => r"Vivaldi\Application\vivaldi.exe",
            Browser::Firefox => r"Mozilla Firefox\firefox.exe",
        };
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let p = PathBuf::from(base).join(rel);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let app = match b {
            Browser::Chrome => "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            Browser::Edge => "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            Browser::Brave => "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            Browser::Chromium => "/Applications/Chromium.app/Contents/MacOS/Chromium",
            Browser::Vivaldi => "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
            Browser::Firefox => "/Applications/Firefox.app/Contents/MacOS/firefox",
        };
        let p = PathBuf::from(app);
        p.is_file().then_some(p)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let names: &[&str] = match b {
            Browser::Chrome => &["google-chrome", "google-chrome-stable"],
            Browser::Edge => &["microsoft-edge", "microsoft-edge-stable"],
            Browser::Brave => &["brave-browser", "brave"],
            Browser::Chromium => &["chromium", "chromium-browser"],
            Browser::Vivaldi => &["vivaldi", "vivaldi-stable"],
            Browser::Firefox => &["firefox", "firefox-esr"],
        };
        let path = std::env::var_os("PATH")?;
        names.iter().flat_map(|n| std::env::split_paths(&path).map(move |d| d.join(n))).find(|p| p.is_file())
    }
}

/// Copies a directory tree (used to install the bundled extension package
/// into a stable folder the browser can load).
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod install_tests {
    use super::*;

    #[test]
    fn pages_and_packages() {
        assert_eq!(Browser::Edge.extensions_page(), "edge://extensions/");
        assert_eq!(Browser::Firefox.package(), "firefox");
        assert_eq!(Browser::Brave.package(), "chromium");
        assert_eq!(Browser::parse("brave"), Some(Browser::Brave));
        assert_eq!(Browser::parse("safari"), None);
    }

    #[test]
    fn copies_trees() {
        let a = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(a.path().join("x/y")).unwrap();
        std::fs::write(a.path().join("x/y/f.txt"), b"hi").unwrap();
        std::fs::write(a.path().join("manifest.json"), b"{}").unwrap();
        let b = tempfile::tempdir().unwrap();
        copy_dir(a.path(), &b.path().join("out")).unwrap();
        assert_eq!(std::fs::read(b.path().join("out/x/y/f.txt")).unwrap(), b"hi");
        assert!(b.path().join("out/manifest.json").exists());
    }
}
