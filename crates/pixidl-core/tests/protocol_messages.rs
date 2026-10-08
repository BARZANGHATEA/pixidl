//! Browser protocol messages handled by a real manager: probe_links,
//! open_in_app and extension client tracking.

mod common;

use common::*;
use pixidl_core::protocol;
use pixidl_core::types::{EngineKind, ManagerEvent};
use serde_json::{json, Value};

async fn call(h: &Harness, v: Value) -> Value {
    protocol::handle(&h.mgr, v.to_string().as_bytes()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_links_reports_names_sizes_and_errors() {
    let srv = start_server(4096, 0).await;
    let h = harness_with(|_| {}).await;
    let r = call(
        &h,
        json!({"version":1,"type":"probe_links","client":{"browser":"chrome","version":"2.0.0"},"payload":{"items":[
            {"url": srv.url("/cd")},
            {"url": srv.url("/file/archive.zip")},
            {"url": srv.url("/missing/gone.iso")},
            {"url": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Linux%20ISO"},
            {"url": "https://www.youtube.com/watch?v=abc"}
        ]}}),
    )
    .await;
    assert_eq!(r["success"], true, "{r}");
    let res = r["results"].as_array().unwrap();
    assert_eq!(res.len(), 5);
    assert_eq!(res[0]["filename"], "report final.pdf");
    assert_eq!(res[0]["totalBytes"], 4096);
    assert_eq!(res[0]["resumable"], true);
    assert_eq!(res[1]["filename"], "archive.zip");
    assert_eq!(res[2]["error"], "File not found");
    assert_eq!(res[2]["filename"], "gone.iso", "name still derived from the URL");
    assert_eq!(res[3]["engine"], "torrent");
    assert_eq!(res[3]["filename"], "Linux ISO");
    assert_eq!(res[4]["engine"], "video");

    // The extension was recorded.
    let clients = h.mgr.extension_clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].browser, "chrome");
    assert_eq!(clients[0].version, "2.0.0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_in_app_emits_show_add_dialog() {
    let h = harness_with(|_| {}).await;
    let r = call(&h, json!({"version":1,"type":"open_in_app","payload":{"url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ"}})).await;
    assert_eq!(r["success"], true, "{r}");
    assert!(h.events.lock().iter().any(|e| matches!(e, ManagerEvent::ShowAddDialog { url } if url.contains("youtube.com/watch"))));
    let bad = call(&h, json!({"version":1,"type":"open_in_app","payload":{"url":"javascript:alert(1)"}})).await;
    assert_eq!(bad["error"]["code"], "invalid_url");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_hint_is_used() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let r = call(&h, json!({"version":1,"type":"add_download","payload":{"url": srv.url("/file/page.bin"), "engine":"http"}})).await;
    assert_eq!(r["engine"], "http");
    let d = h.mgr.get(r["download_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(d.engine, EngineKind::Http);
    let ping = call(&h, json!({"version":1,"type":"ping","client":{"browser":"firefox","version":"2.0.0"}})).await;
    assert_eq!(ping["integration_enabled"], true);
    assert!(h.mgr.extension_clients().iter().any(|c| c.browser == "firefox"));
}
