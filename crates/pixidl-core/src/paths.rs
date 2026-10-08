//! Well-known per-user locations, shared by the app and the native host.

use std::path::PathBuf;

pub const APP_IDENTIFIER: &str = "com.pixidl.app";
pub const NATIVE_HOST_NAME: &str = "com.pixidl.app";

/// `%APPDATA%\com.pixidl.app` on Windows,
/// `~/.local/share/com.pixidl.app` on Linux,
/// `~/Library/Application Support/com.pixidl.app` on macOS.
/// `PIXIDL_DATA_DIR` overrides it (tests, portable installs).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("PIXIDL_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    directories::BaseDirs::new()
        .map(|b| b.data_dir().join(APP_IDENTIFIER))
        .unwrap_or_else(|| std::env::temp_dir().join(APP_IDENTIFIER))
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn bridge_file() -> PathBuf {
    data_dir().join("bridge.json")
}

pub fn engines_dir() -> PathBuf {
    data_dir().join("engines")
}

pub fn native_messaging_dir() -> PathBuf {
    data_dir().join("native-messaging")
}
