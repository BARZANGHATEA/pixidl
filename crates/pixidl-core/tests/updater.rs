//! Updater against a local server that mimics the GitHub releases API and
//! asset downloads (including the redirect GitHub uses for assets).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use parking_lot::Mutex;
use pixidl_core::settings::{ProxyMode, Settings};
use pixidl_core::types::ErrorKind;
use pixidl_core::updater::{HostPolicy, InstallBlocker, UpdateAsset, UpdateEvent, Updater};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const INSTALLER: &str = "pixidl_1.2.0_x64-setup.exe";

#[derive(Clone)]
struct Mock {
    addr: SocketAddr,
    installer: Arc<Vec<u8>>,
    /// Hash published in SHA256SUMS.txt.
    published: Arc<Mutex<String>>,
    requests: Arc<Mutex<Vec<String>>>,
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn releases_json(base: &str) -> serde_json::Value {
    let asset = |tag: &str, name: &str, size: usize| serde_json::json!({ "name": name, "size": size, "browser_download_url": format!("{base}/dl/{tag}/{name}") });
    serde_json::json!([
        { "tag_name": "v3.0.0", "name": "draft", "draft": true, "prerelease": false, "html_url": "x", "assets": [] },
        { "tag_name": "v2.0.0-beta.1", "name": "", "draft": false, "prerelease": true, "body": "beta", "html_url": format!("{base}/rel/v2.0.0-beta.1"), "published_at": "2026-10-05T10:00:00Z", "assets": [] },
        { "tag_name": "v1.2.0", "name": "pixidl v1.2.0", "draft": false, "prerelease": false, "body": "## Changes\n- Faster", "html_url": format!("{base}/rel/v1.2.0"), "published_at": "2026-10-01T10:00:00Z",
          "assets": [asset("v1.2.0", "pixidl-chromium.zip", 10), asset("v1.2.0", INSTALLER, 300 * 1024), asset("v1.2.0", "SHA256SUMS.txt", 200)] },
        { "tag_name": "v1.1.5", "name": "old", "draft": false, "prerelease": false, "html_url": "x", "assets": [] }
    ])
}

async fn releases(State(m): State<Mock>, Path(repo): Path<String>) -> Response {
    m.requests.lock().push(format!("releases:{repo}"));
    match repo.as_str() {
        "pixidl" => axum::Json(releases_json(&format!("http://{}", m.addr))).into_response(),
        "empty" => axum::Json(serde_json::json!([])).into_response(),
        "limited" => {
            let mut h = HeaderMap::new();
            h.insert("x-ratelimit-remaining", "0".parse().unwrap());
            h.insert("x-ratelimit-reset", "1790000000".parse().unwrap());
            (StatusCode::FORBIDDEN, h, "API rate limit exceeded").into_response()
        }
        "broken" => (StatusCode::INTERNAL_SERVER_ERROR, "oops").into_response(),
        _ => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "message": "Not Found" }))).into_response(),
    }
}

async fn asset(State(m): State<Mock>, Path((tag, name)): Path<(String, String)>) -> Response {
    m.requests.lock().push(format!("asset:{tag}/{name}"));
    // GitHub redirects asset downloads to its CDN; mimic the hop.
    Redirect::temporary(&format!("/cdn/{name}")).into_response()
}

async fn cdn(State(m): State<Mock>, Path(name): Path<String>) -> Response {
    m.requests.lock().push(format!("cdn:{name}"));
    match name.as_str() {
        "SHA256SUMS.txt" => format!("{}  pixidl-chromium.zip\r\n{}  {INSTALLER}\r\n", "0".repeat(64), m.published.lock()).into_response(),
        INSTALLER => ([("content-type", "application/octet-stream")], m.installer.as_ref().clone()).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn offsite(State(m): State<Mock>) -> Response {
    // A redirect to a host the policy does not trust ("localhost" ≠ "127.0.0.1").
    Redirect::temporary(&format!("http://localhost:{}/cdn/{INSTALLER}", m.addr.port())).into_response()
}

async fn start() -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let installer: Vec<u8> = (0..300 * 1024u32).map(|i| (i * 31 % 251) as u8).collect();
    let m = Mock { addr, published: Arc::new(Mutex::new(sha(&installer))), installer: Arc::new(installer), requests: Default::default() };
    let app = Router::new()
        .route("/repos/o/:repo/releases", get(releases))
        .route("/dl/:tag/:name", get(asset))
        .route("/cdn/:name", get(cdn))
        .route("/offsite", get(offsite))
        .with_state(m.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    m
}

fn loopback() -> HostPolicy {
    HostPolicy { hosts: vec!["127.0.0.1".into()], redirect_hosts: vec![], https_only: false }
}

fn updater(m: &Mock, repo: &str) -> Updater {
    let s = Settings { proxy_mode: ProxyMode::None, ..Default::default() };
    Updater::new(&s, loopback(), format!("http://{}/repos/o/{repo}", m.addr)).unwrap()
}

#[tokio::test]
async fn finds_the_newest_stable_release_with_installer_and_checksum() {
    let m = start().await;
    let info = updater(&m, "pixidl").check("1.1.0", false).await.unwrap();
    assert!(info.update_available);
    assert_eq!(info.current, "1.1.0");
    let l = info.latest.unwrap();
    assert_eq!(l.version, "1.2.0");
    assert_eq!(l.tag, "v1.2.0");
    assert_eq!(l.name, "pixidl v1.2.0");
    assert_eq!(l.notes, "## Changes\n- Faster");
    assert!(!l.prerelease);
    assert_eq!(l.published_at.as_deref(), Some("2026-10-01T10:00:00Z"));
    let inst = l.installer.unwrap();
    assert_eq!(inst.name, INSTALLER);
    assert_eq!(inst.size, 300 * 1024);
    assert_eq!(l.sha256.unwrap(), sha(&m.installer));
    // Not newer than the running version → no update.
    let info = updater(&m, "pixidl").check("1.2.0", false).await.unwrap();
    assert!(!info.update_available);
    assert!(info.latest.is_some());
}

#[tokio::test]
async fn offers_prereleases_only_when_asked() {
    let m = start().await;
    let info = updater(&m, "pixidl").check("1.2.0", true).await.unwrap();
    assert!(info.update_available);
    let l = info.latest.unwrap();
    assert_eq!(l.version, "2.0.0-beta.1");
    assert!(l.prerelease);
    assert_eq!(l.name, "v2.0.0-beta.1", "falls back to the tag when the name is empty");
    assert!(l.installer.is_none());
    assert!(l.sha256.is_none());
    let expected = if cfg!(windows) { InstallBlocker::NoInstaller } else { InstallBlocker::UnsupportedPlatform };
    assert_eq!(info.install_blocker, Some(expected));
}

#[tokio::test]
async fn no_releases_and_missing_repo_mean_no_update() {
    let m = start().await;
    for repo in ["empty", "missing"] {
        let info = updater(&m, repo).check("1.1.0", false).await.unwrap();
        assert!(info.latest.is_none(), "{repo}");
        assert!(!info.update_available);
        assert!(info.install_blocker.is_none());
    }
}

#[tokio::test]
async fn reports_rate_limiting_and_server_errors() {
    let m = start().await;
    let e = updater(&m, "limited").check("1.1.0", false).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ServerRejected);
    assert!(e.message.contains("limit"), "{}", e.message);
    let e = updater(&m, "broken").check("1.1.0", false).await.unwrap_err();
    assert!(e.detail.unwrap().contains("500"));
}

#[tokio::test]
async fn offline_is_a_network_error() {
    // Nothing listens on this port.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let s = Settings { proxy_mode: ProxyMode::None, ..Default::default() };
    let u = Updater::new(&s, loopback(), format!("http://127.0.0.1:{port}/repos/o/pixidl")).unwrap();
    let e = u.check("1.1.0", false).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::NetworkUnavailable);
}

#[tokio::test]
async fn downloads_and_verifies_the_installer() {
    let m = start().await;
    let up = updater(&m, "pixidl");
    let l = up.check("1.1.0", false).await.unwrap().latest.unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pixidl_1.1.9_x64-setup.exe"), b"stale").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let path = up
        .download(l.installer.as_ref().unwrap(), l.sha256.as_deref(), dir.path(), &move |e| ev.lock().push(e), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(path, dir.path().join(INSTALLER));
    assert_eq!(sha(&std::fs::read(&path).unwrap()), sha(&m.installer));
    assert!(!dir.path().join(format!("{INSTALLER}.part")).exists());
    assert!(!dir.path().join("pixidl_1.1.9_x64-setup.exe").exists(), "old installers are cleaned up");
    let events = events.lock().clone();
    assert!(events.contains(&UpdateEvent::Verifying));
    assert!(events.iter().any(|e| matches!(e, UpdateEvent::Progress { downloaded, total: Some(t), .. } if *downloaded == *t && *t == 300 * 1024)));
    // The request went through the GitHub-style redirect.
    assert!(m.requests.lock().iter().any(|r| r == &format!("cdn:{INSTALLER}")));
    pixidl_core::updater::verify_file(&path, l.sha256.as_deref().unwrap()).unwrap();

    // A second call reuses the verified file without downloading again.
    let before = m.requests.lock().len();
    up.download(l.installer.as_ref().unwrap(), l.sha256.as_deref(), dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap();
    assert_eq!(m.requests.lock().len(), before);
    // Tampering after verification is caught before execution.
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(pixidl_core::updater::verify_file(&path, l.sha256.as_deref().unwrap()).unwrap_err().kind, ErrorKind::ChecksumMismatch);
}

#[tokio::test]
async fn checksum_mismatch_is_refused_and_deleted() {
    let m = start().await;
    *m.published.lock() = "f".repeat(64);
    let up = updater(&m, "pixidl");
    let l = up.check("1.1.0", false).await.unwrap().latest.unwrap();
    assert_eq!(l.sha256.as_deref(), Some("f".repeat(64).as_str()));
    let dir = tempfile::tempdir().unwrap();
    let e = up.download(l.installer.as_ref().unwrap(), l.sha256.as_deref(), dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ChecksumMismatch);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing is left behind");
}

#[tokio::test]
async fn refuses_without_checksum_untrusted_hosts_and_redirects() {
    let m = start().await;
    let up = updater(&m, "pixidl");
    let dir = tempfile::tempdir().unwrap();
    let base = format!("http://{}", m.addr);
    let asset = |url: String| UpdateAsset { name: INSTALLER.into(), url, size: 0 };
    let good = asset(format!("{base}/dl/v1.2.0/{INSTALLER}"));
    let hash = sha(&m.installer);

    // No checksum: nothing is even requested.
    let n = m.requests.lock().len();
    let e = up.download(&good, None, dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::ChecksumMismatch);
    assert_eq!(m.requests.lock().len(), n);

    // A URL outside the policy.
    let e = up.download(&asset("https://example.com/x.exe".into()), Some(&hash), dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::PermissionDenied);

    // A redirect outside the policy.
    let e = up.download(&asset(format!("{base}/offsite")), Some(&hash), dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::PermissionDenied, "{e:?}");
    assert!(!m.requests.lock().iter().any(|r| r.starts_with("cdn:")), "the off-site target was never fetched");

    // A path-like asset name.
    let e = up.download(&UpdateAsset { name: "../evil_x64-setup.exe".into(), ..good.clone() }, Some(&hash), dir.path(), &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidUrl);

    // Cancelled.
    let c = CancellationToken::new();
    c.cancel();
    let e = up.download(&good, Some(&hash), dir.path(), &|_| {}, &c).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// Talks to the real GitHub API; run by hand:
/// `cargo test -p pixidl-core --test updater -- --ignored live_github`
#[tokio::test]
#[ignore = "needs network access to api.github.com"]
async fn live_github_check() {
    let s = Settings::default();
    let info = Updater::github(&s).unwrap().check(pixidl_core::APP_VERSION, true).await.unwrap();
    println!("{info:#?}");
    if let Some(l) = &info.latest {
        assert!(l.html_url.starts_with(pixidl_core::updater::REPO_URL));
    }
}

/// An older app sees the newest real release, and its installer can be
/// verified (SHA256SUMS.txt or GitHub's asset digest), downloaded and checked.
/// `cargo test -p pixidl-core --test updater -- --ignored live_github_download`
#[tokio::test]
#[ignore = "needs network access to github.com"]
async fn live_github_download() {
    let s = Settings::default();
    let up = Updater::github(&s).unwrap();
    let info = up.check("0.0.1", false).await.unwrap();
    println!("{info:#?}");
    let latest = info.latest.expect("a published release");
    let sha = latest.sha256.clone().expect("installer checksum");
    let asset = latest.installer.clone().expect("installer asset");
    let dir = tempfile::tempdir().unwrap();
    let cancel = pixidl_core::updater::CancellationToken::new();
    let path = up.download(&asset, Some(&sha), dir.path(), &|_| {}, &cancel).await.unwrap();
    pixidl_core::updater::verify_file(&path, &sha).unwrap();
    println!("verified {}", path.display());
}

#[test]
fn github_policy_rejects_a_non_github_api() {
    let s = Settings::default();
    assert!(Updater::new(&s, HostPolicy::github(), "https://example.com/repos/a/b").is_err());
    assert!(Updater::new(&s, HostPolicy::github(), "http://api.github.com/repos/a/b").is_err());
    assert!(Updater::github(&s).is_ok());
}
