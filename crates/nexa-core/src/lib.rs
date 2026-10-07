//! Nexa Download Manager core.
//!
//! This crate contains everything that does not depend on the desktop shell:
//! persistence, settings, the download engines, the queue/manager and the
//! browser-integration protocol. It is fully testable headless.

pub mod db;
pub mod detector;
pub mod engines;
pub mod manager;
pub mod error;
pub mod ratelimit;
pub mod scheduler;
pub mod security;
pub mod settings;
pub mod tools;
pub mod types;

pub use error::{DownloadError, Result};
pub const APP_NAME: &str = "Nexa Download Manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
