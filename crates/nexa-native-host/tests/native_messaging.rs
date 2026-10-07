//! End-to-end: browser framing → nexa-native-host process → loopback bridge →
//! DownloadManager → real HTTP download.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use nexa_core::bridge::{BridgeClient, BridgeInfo, BridgeServer};
use nexa_core::db::Db;
use nexa_core::manager::{DownloadManager, ManagerConfig};
use nexa_core::settings::Settings;
use serde_json::{json, Value};

struct Host {
    child: std::process::Child,
}

impl Host {
    fn spawn(data_dir: &std::path::Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_nexa-native-host"))
            .arg("chrome-extension://ndlafmjbcbcjmkegfelbhgknmajgdbna/")
            .env("NEXA_DATA_DIR", data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self { child }
    }

    fn send_raw(&mut self, data: &[u8]) -> Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(&(data.len() as u32).to_ne_bytes()).unwrap();
        stdin.write_all(data).unwrap();
        stdin.flush().unwrap();
        let stdout = self.child.stdout.as_mut().unwrap();
        let mut len = [0u8; 4];
        stdout.read_exact(&mut len).unwrap();
        let mut buf = vec![0u8; u32::from_ne_bytes(len) as usize];
        stdout.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    fn send(&mut self, v: Value) -> Value {
        self.send_raw(v.to_string().as_bytes())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

async fn file_server() -> String {
    let app = axum::Router::new().route("/files/report.pdf", axum::routing::get(|| async { vec![7u8; 100_000] }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/files/report.pdf")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn extension_to_app_round_trip() {
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let db = Db::open(&data.path().join("nexa.db")).unwrap();
    db.save_settings(&Settings { default_download_dir: downloads.path().to_string_lossy().into(), ..Default::default() }).unwrap();
    let mgr = DownloadManager::start_with_db(db, ManagerConfig { data_dir: data.path().into(), tool_dirs: vec![] }, Arc::new(|_| {}))
        .await
        .unwrap();
    let _bridge = BridgeServer::start(mgr.clone(), &data.path().join("bridge.json")).await.unwrap();
    let url = file_server().await;

    let data_path = data.path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let mut host = Host::spawn(&data_path);
        let pong = host.send(json!({"version":1,"type":"ping","id":"p1"}));
        assert_eq!(pong["success"], true, "{pong}");
        assert_eq!(pong["id"], "p1");
        assert_eq!(pong["protocol_version"], 1);

        // Malformed and invalid messages are rejected by the host itself.
        let bad = host.send_raw(b"{oops");
        assert_eq!(bad["error"]["code"], "invalid_json");
        let bad = host.send(json!({"version":1,"type":"add_download","payload":{"url":"file:///etc/passwd"}}));
        assert_eq!(bad["error"]["code"], "invalid_url");
        let bad = host.send(json!({"version":9,"type":"ping"}));
        assert_eq!(bad["error"]["code"], "unsupported_version");

        // A path in the filename is stripped; the folder is never chosen by the browser.
        let added = host.send(json!({"version":1,"type":"add_download","id":"a","payload":{"url":url,"filename":"..\\..\\evil.pdf","referrer":"https://example.com/"}}));
        assert_eq!(added["success"], true, "{added}");
        let id = added["download_id"].as_str().unwrap().to_string();
        assert_eq!(added["filename"], "evil.pdf");

        let mut status = Value::Null;
        for _ in 0..100 {
            status = host.send(json!({"version":1,"type":"get_status","payload":{"download_id":id}}));
            if status["download"]["status"] == "completed" {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(status["download"]["status"], "completed", "{status}");
        assert!(status["download"].get("save_dir").is_none(), "no local paths are exposed");

        let multi = host.send(json!({"version":1,"type":"add_multiple_downloads","payload":{"items":[{"url":format!("{url}?a")},{"url":format!("{url}?b")}]}}));
        assert_eq!(multi["added"], 2, "{multi}");

        let missing = host.send(json!({"version":1,"type":"pause","payload":{"download_id":"nope"}}));
        assert_eq!(missing["error"]["code"], "not_found");
        id
    })
    .await
    .unwrap();
    assert!(downloads.path().join("evil.pdf").exists(), "downloaded into the default folder");
    assert_eq!(mgr.get(&result).unwrap().unwrap().status, nexa_core::types::DownloadStatus::Completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bridge_rejects_wrong_token_and_http() {
    let data = tempfile::tempdir().unwrap();
    let db = Db::open(&data.path().join("nexa.db")).unwrap();
    let mgr = DownloadManager::start_with_db(db, ManagerConfig { data_dir: data.path().into(), tool_dirs: vec![] }, Arc::new(|_| {}))
        .await
        .unwrap();
    let info_path = data.path().join("bridge.json");
    let server = BridgeServer::start(mgr, &info_path).await.unwrap();
    let info: BridgeInfo = nexa_core::bridge::read_info(&info_path).unwrap();
    assert_eq!(info.token.len(), 64);
    assert_eq!(server.addr.ip().to_string(), "127.0.0.1");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&info_path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    let mut wrong = info.clone();
    wrong.token = "0".repeat(64);
    assert!(BridgeClient::connect(&wrong).await.is_err());

    // A browser page doing fetch('http://127.0.0.1:port') gets nothing useful.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", info.port)).await.unwrap();
    s.write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}").await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(6), s.read_to_end(&mut buf)).await;
    assert!(!String::from_utf8_lossy(&buf).contains("HTTP/1.1 200"));

    let mut ok = BridgeClient::connect(&info).await.unwrap();
    let r: Value = serde_json::from_slice(&ok.request(br#"{"version":1,"type":"ping"}"#).await.unwrap()).unwrap();
    assert_eq!(r["success"], true);

    drop(server);
    assert!(!info_path.exists(), "bridge.json removed on shutdown");
}

#[test]
fn app_unavailable_is_reported() {
    let data = tempfile::tempdir().unwrap();
    let mut host = Host::spawn(data.path());
    let r = host.send(json!({"version":1,"type":"ping","id":7}));
    assert_eq!(r["success"], false);
    assert_eq!(r["error"]["code"], "app_unavailable");
    assert_eq!(r["id"], "7");
}

#[test]
fn register_and_unregister_manifests() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let run = |arg: &str| {
        Command::new(env!("CARGO_BIN_EXE_nexa-native-host"))
            .args([arg, "--extension-id", "abcdefghijklmnopabcdefghijklmnop"])
            .env("HOME", home.path())
            .env("NEXA_DATA_DIR", data.path())
            .output()
            .unwrap()
    };
    let out = run("--register");
    assert!(out.status.success());
    let res: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(res.as_array().unwrap().iter().all(|r| r["registered"] == true), "{res}");
    #[cfg(target_os = "linux")]
    {
        let m: Value = serde_json::from_slice(&std::fs::read(home.path().join(".config/google-chrome/NativeMessagingHosts/com.nexa.downloadmanager.json")).unwrap()).unwrap();
        assert_eq!(m["path"], env!("CARGO_BIN_EXE_nexa-native-host"));
        assert_eq!(m["allowed_origins"].as_array().unwrap().len(), 2);
        let ff: Value = serde_json::from_slice(&std::fs::read(home.path().join(".mozilla/native-messaging-hosts/com.nexa.downloadmanager.json")).unwrap()).unwrap();
        assert_eq!(ff["allowed_extensions"][0], "nexa@nexa-download-manager.app");
    }
    let st: Value = serde_json::from_slice(&run("--status").stdout).unwrap();
    assert!(st.as_array().unwrap().iter().all(|r| r["registered"] == true), "{st}");
    assert!(run("--unregister").status.success());
    let st: Value = serde_json::from_slice(&run("--status").stdout).unwrap();
    assert!(st.as_array().unwrap().iter().all(|r| r["registered"] == false));
}
