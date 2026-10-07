use std::sync::atomic::AtomicBool;

use nexa_core::bridge::BridgeServer;
use nexa_core::manager::DownloadManager;
use parking_lot::Mutex;

pub struct AppState {
    pub mgr: DownloadManager,
    pub bridge: Mutex<Option<BridgeServer>>,
    pub power_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub quitting: AtomicBool,
}
