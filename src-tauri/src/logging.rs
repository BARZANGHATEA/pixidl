//! Structured local logging: JSON lines in `<data>/logs/nexa.log.YYYY-MM-DD`,
//! rotated daily, keeping the last 7 files. URLs are redacted before logging
//! by the core; no cookies, tokens or headers are ever logged.

use std::path::Path;

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init(logs_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let _ = std::fs::create_dir_all(logs_dir);
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("nexa")
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("NEXA_LOG").unwrap_or_else(|_| EnvFilter::new("info,librqbit=warn,librqbit_dht=warn,tracker_comms=warn,hyper=warn,reqwest=warn"));
    let file_layer = fmt::layer().json().with_writer(writer).with_current_span(false);
    let registry = tracing_subscriber::registry().with(filter).with(file_layer);
    if cfg!(debug_assertions) {
        let _ = registry.with(fmt::layer().with_writer(std::io::stderr)).try_init();
    } else {
        let _ = registry.try_init();
    }
    Some(guard)
}
