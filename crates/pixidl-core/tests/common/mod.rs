#![allow(dead_code)]
//! Test harness: a local HTTP server with controllable behaviour, and helpers
//! to run a real DownloadManager against it.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use pixidl_core::db::Db;
use pixidl_core::manager::{DownloadManager, ManagerConfig};
use pixidl_core::settings::Settings;
use pixidl_core::types::{Download, DownloadStatus, ManagerEvent};
use parking_lot::Mutex;

pub fn payload(len: usize) -> Vec<u8> {
    // Deterministic, non-repeating-ish content so misplaced bytes are detected.
    let mut v = Vec::with_capacity(len);
    let mut x: u32 = 0x1234_5678;
    for _ in 0..len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        v.push((x & 0xff) as u8);
    }
    v
}

#[derive(Clone)]
pub struct ServerState {
    pub data: Arc<Vec<u8>>,
    pub ranges: Arc<Mutex<Vec<Option<String>>>>,
    pub flaky_failures: Arc<AtomicU32>,
    pub chunk_delay_ms: u64,
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub state: ServerState,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn ranges(&self) -> Vec<Option<String>> {
        self.state.ranges.lock().clone()
    }
}

fn parse_range(h: &HeaderMap, len: u64) -> Option<(u64, u64)> {
    let v = h.get("range")?.to_str().ok()?;
    let spec = v.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let a: u64 = a.parse().ok()?;
    let b: u64 = if b.is_empty() { len - 1 } else { b.parse().ok()? };
    Some((a, b.min(len - 1)))
}

fn respond(state: &ServerState, headers: &HeaderMap, ranged: bool, delay_ms: u64, extra: &[(&str, &str)]) -> Response<Body> {
    state.ranges.lock().push(headers.get("range").and_then(|v| v.to_str().ok()).map(String::from));
    let len = state.data.len() as u64;
    let (status, start, end) = match (ranged, parse_range(headers, len)) {
        (true, Some((a, _))) if a >= len => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("content-range", format!("bytes */{len}"))
                .body(Body::empty())
                .unwrap();
        }
        (true, Some((a, b))) => (StatusCode::PARTIAL_CONTENT, a, b),
        _ => (StatusCode::OK, 0, len - 1),
    };
    let slice = state.data[start as usize..=end as usize].to_vec();
    let mut b = Response::builder().status(status).header("content-length", slice.len().to_string()).header("etag", "\"v1\"");
    if ranged {
        b = b.header("accept-ranges", "bytes");
        if status == StatusCode::PARTIAL_CONTENT {
            b = b.header("content-range", format!("bytes {start}-{end}/{len}"));
        }
    }
    for (k, v) in extra {
        b = b.header(*k, HeaderValue::from_str(v).unwrap());
    }
    let body = if delay_ms == 0 {
        Body::from(slice)
    } else {
        let chunks: Vec<Vec<u8>> = slice.chunks(16 * 1024).map(|c| c.to_vec()).collect();
        let stream = futures::stream::iter(chunks).then(move |c| async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            Ok::<_, std::io::Error>(bytes::Bytes::from(c))
        });
        Body::from_stream(stream)
    };
    b.body(body).unwrap()
}

use futures::StreamExt;

pub async fn start_server(size: usize, chunk_delay_ms: u64) -> TestServer {
    let state = ServerState {
        data: Arc::new(payload(size)),
        ranges: Default::default(),
        flaky_failures: Arc::new(AtomicU32::new(2)),
        chunk_delay_ms,
    };
    let app = Router::new()
        .route("/file/:name", get(|State(s): State<ServerState>, h: HeaderMap| async move { respond(&s, &h, true, 0, &[]) }))
        .route("/slow/:name", get(|State(s): State<ServerState>, h: HeaderMap| async move { let d = s.chunk_delay_ms; respond(&s, &h, true, d, &[]) }))
        .route("/norange/:name", get(|State(s): State<ServerState>, h: HeaderMap| async move { let d = s.chunk_delay_ms; respond(&s, &h, false, d, &[]) }))
        .route(
            "/cd",
            get(|State(s): State<ServerState>, h: HeaderMap| async move {
                respond(&s, &h, true, 0, &[("content-disposition", "attachment; filename=\"report final.pdf\"")])
            }),
        )
        .route(
            "/flaky/:name",
            get(|State(s): State<ServerState>, h: HeaderMap| async move {
                if s.flaky_failures.load(Ordering::SeqCst) > 0 {
                    s.flaky_failures.fetch_sub(1, Ordering::SeqCst);
                    return Response::builder().status(503).body(Body::empty()).unwrap();
                }
                respond(&s, &h, true, 0, &[])
            }),
        )
        .route("/missing/:name", get(|| async { StatusCode::NOT_FOUND }))
        .route("/page", get(|| async { ([("content-type", "text/html")], "<html><body>hi</body></html>") }))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    TestServer { addr, state }
}

pub struct Harness {
    pub mgr: DownloadManager,
    pub dir: tempfile::TempDir,
    pub data_dir: tempfile::TempDir,
    pub events: Arc<Mutex<Vec<ManagerEvent>>>,
}

pub fn test_settings(dir: &std::path::Path) -> Settings {
    Settings {
        default_download_dir: dir.to_string_lossy().to_string(),
        retry_delay_secs: 1,
        retry_count: 3,
        connect_timeout_secs: 5,
        read_timeout_secs: 10,
        proxy_mode: pixidl_core::settings::ProxyMode::None,
        ..Default::default()
    }
}

pub async fn harness_with(f: impl FnOnce(&mut Settings)) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let db = Db::open(&data_dir.path().join("pixidl.db")).unwrap();
    let mut s = test_settings(dir.path());
    f(&mut s);
    db.save_settings(&s).unwrap();
    harness_from(db, dir, data_dir).await
}

pub async fn harness_from(db: Db, dir: tempfile::TempDir, data_dir: tempfile::TempDir) -> Harness {
    let events: Arc<Mutex<Vec<ManagerEvent>>> = Default::default();
    let ev = events.clone();
    let mgr = DownloadManager::start_with_db(
        db,
        ManagerConfig { data_dir: data_dir.path().to_path_buf(), tool_dirs: vec![] },
        Arc::new(move |e: ManagerEvent| ev.lock().push(e)),
    )
    .await
    .unwrap();
    Harness { mgr, dir, data_dir, events }
}

impl Harness {
    pub async fn wait_for(&self, id: &str, timeout_s: u64, pred: impl Fn(&Download) -> bool) -> Download {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_s);
        loop {
            let d = self.mgr.get(id).unwrap().expect("download exists");
            if pred(&d) {
                return d;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timeout waiting; last state: {:?} {:?} {:?} {:?}", d.status, d.error_kind, d.error_message, d.error_detail);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn wait_status(&self, id: &str, status: DownloadStatus, timeout_s: u64) -> Download {
        self.wait_for(id, timeout_s, |d| d.status == status).await
    }
}

// ------------------------------------------------------------------ range server

/// Behaviour of [`start_range_server`].
#[derive(Clone, Copy, Debug)]
pub struct RangeServerOpts {
    pub size: usize,
    /// Speed of every response (bytes/s per connection); 0 = unthrottled.
    pub bytes_per_sec: u64,
    /// Responses that start at offset 0 are this many times slower.
    pub slow_zero_factor: u64,
    /// Answer 503 while this many responses are already streaming (0 = no limit).
    pub conn_limit: u32,
}

impl Default for RangeServerOpts {
    fn default() -> Self {
        Self { size: 1024 * 1024, bytes_per_sec: 0, slow_zero_factor: 1, conn_limit: 0 }
    }
}

/// A ranged file server that throttles each connection and records
/// concurrency, for testing multi-connection downloads.
#[derive(Clone)]
pub struct RangeServer {
    pub addr: SocketAddr,
    pub data: Arc<Vec<u8>>,
    /// `(start, end_inclusive)` of every served or rejected request.
    pub requests: Arc<Mutex<Vec<(u64, u64)>>>,
    pub inflight: Arc<AtomicU32>,
    pub max_inflight: Arc<AtomicU32>,
    pub rejected: Arc<AtomicU32>,
    opts: RangeServerOpts,
}

impl RangeServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
    pub fn requests(&self) -> Vec<(u64, u64)> {
        self.requests.lock().clone()
    }
    pub fn inflight(&self) -> u32 {
        self.inflight.load(Ordering::SeqCst)
    }
}

struct InflightGuard(Arc<AtomicU32>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn range_response(s: &RangeServer, h: &HeaderMap) -> Response<Body> {
    let len = s.data.len() as u64;
    let (start, end) = parse_range(h, len).unwrap_or((0, len - 1));
    s.requests.lock().push((start, end));
    let now = s.inflight.fetch_add(1, Ordering::SeqCst) + 1;
    if s.opts.conn_limit > 0 && now > s.opts.conn_limit {
        s.inflight.fetch_sub(1, Ordering::SeqCst);
        s.rejected.fetch_add(1, Ordering::SeqCst);
        return Response::builder().status(503).body(Body::empty()).unwrap();
    }
    s.max_inflight.fetch_max(now, Ordering::SeqCst);
    let guard = Arc::new(InflightGuard(s.inflight.clone()));
    let ranged = h.contains_key("range");
    let slice = s.data[start as usize..=end as usize].to_vec();
    let mut bps = s.opts.bytes_per_sec;
    if start == 0 {
        bps /= s.opts.slow_zero_factor.max(1);
    }
    let chunk = 16 * 1024;
    let begun = tokio::time::Instant::now();
    let chunks: Vec<Vec<u8>> = slice.chunks(chunk).map(|c| c.to_vec()).collect();
    let stream = futures::stream::iter(chunks.into_iter().enumerate()).then(move |(i, c)| {
        // The body holds the guard for as long as it is alive.
        let g = guard.clone();
        let at = if bps == 0 { begun } else { begun + Duration::from_secs_f64(((i + 1) * chunk) as f64 / bps as f64) };
        async move {
            tokio::time::sleep_until(at).await;
            drop(g);
            Ok::<_, std::io::Error>(bytes::Bytes::from(c))
        }
    });
    let mut b = Response::builder()
        .status(if ranged { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header("content-length", (end - start + 1).to_string())
        .header("accept-ranges", "bytes")
        .header("etag", "\"r1\"");
    if ranged {
        b = b.header("content-range", format!("bytes {start}-{end}/{len}"));
    }
    b.body(Body::from_stream(stream)).unwrap()
}

pub async fn start_range_server(opts: RangeServerOpts) -> RangeServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv = RangeServer {
        addr: listener.local_addr().unwrap(),
        data: Arc::new(payload(opts.size)),
        requests: Default::default(),
        inflight: Default::default(),
        max_inflight: Default::default(),
        rejected: Default::default(),
        opts,
    };
    let app = Router::new().route("/r/:name", get(|State(s): State<RangeServer>, h: HeaderMap| async move { range_response(&s, &h) })).with_state(srv.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    srv
}
