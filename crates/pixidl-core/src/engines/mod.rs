//! Download engine abstraction.
//!
//! The manager never knows *how* a download is performed. It hands an engine a
//! [`JobContext`] and gets back an [`EngineOutcome`] (or a classified error).
//! Engines publish progress into a shared [`ProgressCell`] which the manager
//! samples at a fixed rate (event throttling), and send rare metadata changes
//! through [`JobContext::meta`].

pub mod http;
pub mod torrent;
pub mod video;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use crate::db::Db;
use crate::error::Result;
use crate::ratelimit::RateLimiter;
use crate::settings::Settings;
use crate::types::{Download, EngineKind};

/// Instruction from the manager to a running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Run,
    /// Stop transferring, keep partial data.
    Pause,
    /// Stop transferring; the manager will clean up partial data.
    Cancel,
}

/// Rare metadata updates from an engine.
#[derive(Debug, Clone, Default)]
pub struct MetaUpdate {
    pub filename: Option<String>,
    pub url: Option<String>,
    pub total_bytes: Option<u64>,
    pub resumable: Option<bool>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub info_hash: Option<String>,
    pub connections: Option<u32>,
    /// The engine finished preparing and is transferring data.
    pub downloading: bool,
    /// Informational event for the download's log.
    pub event: Option<String>,
}

/// Live progress, written by the engine and sampled by the manager.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed_bps: u64,
    pub upload_bps: u64,
    pub eta_seconds: Option<u64>,
    pub peers: Option<u32>,
    pub seeds: Option<u32>,
}

#[derive(Default)]
pub struct ProgressCell {
    inner: Mutex<EngineProgress>,
}

impl ProgressCell {
    pub fn set(&self, p: EngineProgress) {
        *self.inner.lock() = p;
    }
    pub fn update(&self, f: impl FnOnce(&mut EngineProgress)) {
        f(&mut self.inner.lock());
    }
    pub fn get(&self) -> EngineProgress {
        self.inner.lock().clone()
    }
}

/// Everything an engine needs to run one download.
pub struct JobContext {
    pub download: Download,
    pub settings: Arc<Settings>,
    pub db: Db,
    pub http: reqwest::Client,
    pub control: watch::Receiver<Control>,
    pub progress: Arc<ProgressCell>,
    pub meta: mpsc::UnboundedSender<MetaUpdate>,
    pub global_limiter: Arc<RateLimiter>,
    pub download_limiter: Arc<RateLimiter>,
}

impl JobContext {
    pub fn control_state(&self) -> Control {
        *self.control.borrow()
    }

    pub fn send_meta(&self, m: MetaUpdate) {
        let _ = self.meta.send(m);
    }

    /// Resolves when the manager asks the job to stop.
    pub async fn stopped(control: &mut watch::Receiver<Control>) -> Control {
        loop {
            let c = *control.borrow_and_update();
            if c != Control::Run {
                return c;
            }
            if control.changed().await.is_err() {
                return Control::Cancel;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineOutcome {
    /// Finished; `filename` is the final name inside `save_dir`.
    Completed { filename: String },
    /// Honoured a Pause/Cancel request.
    Stopped,
}

pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;

    /// Runs (or resumes) a download until it completes, fails or is stopped.
    fn run(&self, ctx: JobContext) -> BoxFuture<'static, Result<EngineOutcome>>;

    /// Removes partial data after a cancel (or remove with delete-files).
    fn cleanup(&self, download: &Download, delete_completed: bool) -> BoxFuture<'static, ()>;
}

/// Sliding-window throughput estimate.
pub struct SpeedMeter {
    samples: std::collections::VecDeque<(Instant, u64)>,
    window: Duration,
}

impl SpeedMeter {
    pub fn new(window: Duration) -> Self {
        Self { samples: Default::default(), window }
    }

    /// Records the cumulative byte count and returns bytes/s.
    pub fn record(&mut self, total: u64) -> u64 {
        let now = Instant::now();
        self.samples.push_back((now, total));
        while let Some(&(t, _)) = self.samples.front() {
            if now.duration_since(t) > self.window && self.samples.len() > 2 {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        let (t0, b0) = *self.samples.front().unwrap();
        let dt = now.duration_since(t0).as_secs_f64();
        if dt < 0.2 {
            return 0;
        }
        (total.saturating_sub(b0) as f64 / dt) as u64
    }
}

pub fn eta(total: Option<u64>, downloaded: u64, speed: u64) -> Option<u64> {
    let total = total?;
    if speed == 0 || downloaded >= total {
        return None;
    }
    Some((total - downloaded).div_ceil(speed))
}

/// Free space on the volume holding `dir` (walks up to an existing ancestor).
pub fn available_space(dir: &std::path::Path) -> Option<u64> {
    let mut p = dir.to_path_buf();
    loop {
        if p.exists() {
            return fs4::available_space(&p).ok();
        }
        if !p.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_calc() {
        assert_eq!(eta(Some(100), 0, 10), Some(10));
        assert_eq!(eta(Some(100), 95, 10), Some(1));
        assert_eq!(eta(None, 0, 10), None);
        assert_eq!(eta(Some(100), 0, 0), None);
    }

    #[test]
    fn available_space_walks_up() {
        let d = tempfile::tempdir().unwrap();
        assert!(available_space(&d.path().join("not/yet/created")).unwrap() > 0);
    }
}
