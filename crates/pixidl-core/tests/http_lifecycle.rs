//! End-to-end download lifecycle tests against a real local HTTP server.

mod common;

use std::time::{Duration, Instant};

use common::*;
use pixidl_core::db::Db;
use pixidl_core::manager::AddSource;
use pixidl_core::types::*;
use sha2::{Digest, Sha256};

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn req(url: String) -> AddDownloadRequest {
    AddDownloadRequest { url, ..Default::default() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_single_connection_file() {
    let srv = start_server(300 * 1024, 0).await;
    let h = harness_with(|_| {}).await;
    let d = h.mgr.add(req(srv.url("/file/small.bin")), AddSource::User).await.unwrap();
    assert_eq!(d.status, DownloadStatus::Queued);
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    let bytes = std::fs::read(h.dir.path().join("small.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data));
    assert_eq!(done.downloaded_bytes, 300 * 1024);
    assert_eq!(done.total_bytes, Some(300 * 1024));
    assert!(!h.dir.path().join("small.bin.part").exists(), "partial file must be finalised");
    assert_eq!(done.resumable, Some(true));
    assert!(h.events.lock().iter().any(|e| matches!(e, ManagerEvent::DownloadCompleted { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_with_multiple_connections() {
    let srv = start_server(9 * 1024 * 1024 + 123, 0).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(req(srv.url("/file/big.bin")), AddSource::User).await.unwrap();
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 60).await;
    assert_eq!(done.connections, 4);
    let bytes = std::fs::read(h.dir.path().join("big.bin")).unwrap();
    assert_eq!(bytes.len(), srv.state.data.len());
    assert_eq!(sha(&bytes), sha(&srv.state.data));
    // Several ranged requests were made (probe + 4 segments).
    let ranged = srv.ranges().iter().filter(|r| r.as_deref().is_some_and(|r| r.contains('-') && !r.ends_with('-'))).count();
    assert!(ranged >= 4, "expected segment requests, got {:?}", srv.ranges());
    assert!(h.mgr.db().load_segments(&d.id).unwrap().is_empty(), "segments cleared after completion");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_continue_from_partial_data() {
    // 1 MiB delivered in 16 KiB chunks every 20 ms ≈ 1.3 s
    let srv = start_server(1024 * 1024, 20).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let d = h.mgr.add(req(srv.url("/slow/pause.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.status == DownloadStatus::Downloading && d.downloaded_bytes > 100 * 1024).await;
    h.mgr.pause(&d.id).unwrap();
    let paused = h.wait_status(&d.id, DownloadStatus::Paused, 10).await;
    assert!(paused.downloaded_bytes > 0 && paused.downloaded_bytes < 1024 * 1024);
    let part = h.dir.path().join("pause.bin.part");
    let part_len = std::fs::metadata(&part).unwrap().len();
    assert!(part_len > 0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::metadata(&part).unwrap().len(), part_len, "no writes while paused");

    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    let bytes = std::fs::read(h.dir.path().join("pause.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data));
    // The resume request asked for the remaining bytes only.
    let resume_range = srv.ranges().last().cloned().flatten().unwrap();
    assert_eq!(resume_range, format!("bytes={part_len}-"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_connection_pause_resume() {
    // 4 MiB over 4 connections, 16 KiB every 25 ms each ≈ 1.6 s
    let srv = start_server(4 * 1024 * 1024, 25).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(req(srv.url("/slow/multi.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.downloaded_bytes > 256 * 1024).await;
    h.mgr.pause(&d.id).unwrap();
    let p = h.wait_status(&d.id, DownloadStatus::Paused, 10).await;
    let segs = h.mgr.db().load_segments(&d.id).unwrap();
    assert_eq!(segs.len(), 4);
    let seg_total: u64 = segs.iter().map(|s| s.downloaded).sum();
    assert!(seg_total > 0 && seg_total <= p.downloaded_bytes + 1);
    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 30).await;
    let bytes = std::fs::read(h.dir.path().join("multi.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_removes_partial_file() {
    let srv = start_server(2 * 1024 * 1024, 20).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let d = h.mgr.add(req(srv.url("/slow/cancel.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.downloaded_bytes > 64 * 1024).await;
    h.mgr.cancel(&d.id).await.unwrap();
    let c = h.wait_status(&d.id, DownloadStatus::Cancelled, 10).await;
    assert_eq!(c.downloaded_bytes, 0);
    assert!(!h.dir.path().join("cancel.bin.part").exists());
    assert!(!h.dir.path().join("cancel.bin").exists());
    // Cancelled downloads can be restarted.
    h.mgr.retry(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 30).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_without_range_support_restarts_honestly() {
    let srv = start_server(1024 * 1024, 25).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(req(srv.url("/norange/nr.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.downloaded_bytes > 64 * 1024).await;
    let running = h.mgr.get(&d.id).unwrap().unwrap();
    assert_eq!(running.resumable, Some(false));
    assert_eq!(running.connections, 1, "no multi-connection without range support");
    h.mgr.pause(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Paused, 10).await;
    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    let bytes = std::fs::read(h.dir.path().join("nr.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data), "restart from zero must not corrupt the file");
    let events = h.mgr.events(&d.id).unwrap();
    assert!(events.iter().any(|e| e.message.as_deref().unwrap_or("").contains("does not support resume")), "{events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transient_errors_are_retried() {
    let srv = start_server(64 * 1024, 0).await;
    let h = harness_with(|s| s.retry_count = 3).await;
    let d = h.mgr.add(req(srv.url("/flaky/f.bin")), AddSource::User).await.unwrap();
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 30).await;
    assert_eq!(done.retry_count, 2);
    let events = h.mgr.events(&d.id).unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "retry_scheduled").count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn permanent_errors_fail_without_retry() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let d = h.mgr.add(req(srv.url("/missing/x.zip")), AddSource::User).await.unwrap();
    let f = h.wait_status(&d.id, DownloadStatus::Failed, 10).await;
    assert_eq!(f.error_kind, Some(ErrorKind::NotFound));
    assert_eq!(f.retry_count, 0);
    assert!(f.error_message.is_some());
    assert!(h.events.lock().iter().any(|e| matches!(e, ManagerEvent::DownloadFailed { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_exhausted_then_failed_then_manual_retry_works() {
    let srv = start_server(1024, 0).await;
    srv.state.flaky_failures.store(3, std::sync::atomic::Ordering::SeqCst);
    let h = harness_with(|s| s.retry_count = 1).await;
    let d = h.mgr.add(req(srv.url("/flaky/x.bin")), AddSource::User).await.unwrap();
    // 503, auto-retry 503 → Failed (budget of 1 retry used up).
    let f = h.wait_status(&d.id, DownloadStatus::Failed, 20).await;
    assert_eq!(f.error_kind, Some(ErrorKind::ServerRejected));
    assert_eq!(f.retry_count, 1);
    // A manual retry resets the budget: 503, auto-retry → success.
    h.mgr.retry(&d.id).unwrap();
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    assert_eq!(done.retry_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_limit_is_enforced() {
    let srv = start_server(256 * 1024, 15).await;
    let h = harness_with(|s| {
        s.max_concurrent_downloads = 2;
        s.connections_per_download = 1;
    })
    .await;
    let mut ids = vec![];
    for i in 0..5 {
        ids.push(h.mgr.add(req(srv.url(&format!("/slow/c{i}.bin"))), AddSource::User).await.unwrap().id);
    }
    let start = Instant::now();
    let mut max_seen = 0;
    loop {
        let list = h.mgr.list().unwrap();
        let active = list.iter().filter(|d| d.status.is_running()).count();
        max_seen = max_seen.max(active);
        if list.iter().all(|d| d.status == DownloadStatus::Completed) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timeout");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(max_seen, 2, "never more than 2 active downloads");
    // QueueFinished comes from the queue pass that follows the last completion.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !h.events.lock().iter().any(|e| matches!(e, ManagerEvent::QueueFinished)) {
        assert!(Instant::now() < deadline, "QueueFinished was not emitted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn priority_and_queue_order() {
    let srv = start_server(64 * 1024, 5).await;
    let h = harness_with(|s| s.max_concurrent_downloads = 1).await;
    let a = h.mgr.add(AddDownloadRequest { url: srv.url("/slow/a.bin"), start_paused: true, ..Default::default() }, AddSource::User).await.unwrap();
    let b = h.mgr.add(AddDownloadRequest { url: srv.url("/slow/b.bin"), start_paused: true, ..Default::default() }, AddSource::User).await.unwrap();
    let c = h.mgr.add(AddDownloadRequest { url: srv.url("/slow/c.bin"), start_paused: true, priority: Some(Priority::High), ..Default::default() }, AddSource::User).await.unwrap();
    h.mgr.reorder(&[b.id.clone(), a.id.clone()]).unwrap();
    h.mgr.resume_all().unwrap();
    for id in [&a.id, &b.id, &c.id] {
        h.wait_status(id, DownloadStatus::Completed, 30).await;
    }
    let start = |id: &str| h.mgr.get(id).unwrap().unwrap().started_at.unwrap();
    assert!(start(&c.id) < start(&b.id), "high priority first");
    assert!(start(&b.id) < start(&a.id), "then queue order");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_speed_limit_is_enforced() {
    let srv = start_server(600 * 1024, 0).await;
    let h = harness_with(|s| {
        s.global_speed_limit_bps = Some(200 * 1024);
        s.connections_per_download = 1;
    })
    .await;
    let t = Instant::now();
    let d = h.mgr.add(req(srv.url("/file/limited.bin")), AddSource::User).await.unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 30).await;
    let e = t.elapsed().as_secs_f64();
    assert!(e >= 2.3, "600 KiB at 200 KiB/s should take ~3 s, took {e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_download_limit_applies_live() {
    let srv = start_server(2 * 1024 * 1024, 0).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let d = h.mgr.add(AddDownloadRequest { url: srv.url("/file/live.bin"), start_paused: true, ..Default::default() }, AddSource::User).await.unwrap();
    h.mgr.set_download_limit(&d.id, Some(256 * 1024)).unwrap();
    h.mgr.resume(&d.id).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mid = h.mgr.get(&d.id).unwrap().unwrap();
    assert!(mid.downloaded_bytes < 1024 * 1024, "limited: {}", mid.downloaded_bytes);
    h.mgr.set_download_limit(&d.id, None).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn content_disposition_and_duplicates() {
    let srv = start_server(10 * 1024, 0).await;
    let h = harness_with(|_| {}).await;
    std::fs::write(h.dir.path().join("report final.pdf"), b"existing").unwrap();
    let d = h.mgr.add(req(srv.url("/cd")), AddSource::User).await.unwrap();
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 10).await;
    assert_eq!(done.filename, "report final (1).pdf");
    assert_eq!(std::fs::read(h.dir.path().join("report final.pdf")).unwrap(), b"existing", "existing file untouched");
    assert_eq!(done.category, "General"); // provisional name "cd" had no extension
    // An explicit filename is kept.
    let e = h.mgr.add(AddDownloadRequest { url: srv.url("/cd?x=2"), filename: Some("../mine.bin".into()), ..Default::default() }, AddSource::User).await.unwrap();
    let e = h.wait_status(&e.id, DownloadStatus::Completed, 10).await;
    assert_eq!(e.filename, "mine.bin");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn destination_must_be_approved() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let elsewhere = tempfile::tempdir().unwrap();
    let r = h.mgr.add(AddDownloadRequest { url: srv.url("/file/a.bin"), save_dir: Some(elsewhere.path().to_string_lossy().into()), ..Default::default() }, AddSource::User).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::PermissionDenied);
    // A subfolder of the download folder is fine.
    let sub = h.dir.path().join("sub");
    let ok = h.mgr.add(AddDownloadRequest { url: srv.url("/file/a.bin"), save_dir: Some(sub.to_string_lossy().into()), ..Default::default() }, AddSource::User).await.unwrap();
    h.wait_status(&ok.id, DownloadStatus::Completed, 10).await;
    assert!(sub.join("a.bin").exists());
    // The browser can never choose the folder.
    let b = h.mgr.add(AddDownloadRequest { url: srv.url("/file/b.bin"), save_dir: Some(elsewhere.path().to_string_lossy().into()), ..Default::default() }, AddSource::Browser).await.unwrap();
    assert_eq!(b.save_dir, h.dir.path().to_string_lossy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_inputs_rejected() {
    let h = harness_with(|_| {}).await;
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "", "not a url", "ftp://example.com/x"] {
        assert!(h.mgr.add(req(bad.into()), AddSource::User).await.is_err(), "{bad}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn duplicate_active_url_is_not_added_twice() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let a = h.mgr.add(AddDownloadRequest { url: srv.url("/file/same.bin"), start_paused: true, ..Default::default() }, AddSource::Browser).await.unwrap();
    let b = h.mgr.add(AddDownloadRequest { url: srv.url("/file/same.bin"), ..Default::default() }, AddSource::Browser).await.unwrap();
    assert_eq!(a.id, b.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_transitions_are_rejected() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let d = h.mgr.add(req(srv.url("/file/t.bin")), AddSource::User).await.unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 10).await;
    // Pausing/cancelling a completed download is a no-op, never a state change.
    h.mgr.pause(&d.id).unwrap();
    h.mgr.cancel(&d.id).await.unwrap();
    assert_eq!(h.mgr.get(&d.id).unwrap().unwrap().status, DownloadStatus::Completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_keeps_completed_file_unless_asked() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let a = h.mgr.add(req(srv.url("/file/keep.bin")), AddSource::User).await.unwrap();
    let b = h.mgr.add(req(srv.url("/file/del.bin")), AddSource::User).await.unwrap();
    h.wait_status(&a.id, DownloadStatus::Completed, 10).await;
    h.wait_status(&b.id, DownloadStatus::Completed, 10).await;
    h.mgr.remove(&a.id, false).await.unwrap();
    h.mgr.remove(&b.id, true).await.unwrap();
    assert!(h.dir.path().join("keep.bin").exists());
    assert!(!h.dir.path().join("del.bin").exists());
    assert!(h.mgr.get(&a.id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clear_history_keeps_files() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let a = h.mgr.add(req(srv.url("/file/h.bin")), AddSource::User).await.unwrap();
    h.wait_status(&a.id, DownloadStatus::Completed, 10).await;
    h.mgr.clear_history().unwrap();
    assert!(h.mgr.list().unwrap().is_empty());
    assert!(h.dir.path().join("h.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_recovery_resumes_from_disk() {
    let srv = start_server(512 * 1024, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let db_path = data_dir.path().join("pixidl.db");
    let id;
    {
        // Simulate a crash: the DB says "downloading" and a partial file exists.
        let db = Db::open(&db_path).unwrap();
        let mut s = test_settings(dir.path());
        s.connections_per_download = 1;
        db.save_settings(&s).unwrap();
        let t = pixidl_core::db::now();
        let d = Download {
            id: "crashed".into(),
            url: srv.url("/file/crash.bin"),
            original_url: srv.url("/file/crash.bin"),
            referrer: None,
            filename: "crash.bin".into(),
            save_dir: dir.path().to_string_lossy().into(),
            category: "General".into(),
            engine: EngineKind::Http,
            status: DownloadStatus::Downloading,
            priority: Priority::Normal,
            queue_position: 1,
            total_bytes: Some(512 * 1024),
            downloaded_bytes: 400 * 1024, // stale: more than on disk
            speed_bps: 1000,
            upload_bps: 0,
            eta_seconds: Some(5),
            resumable: Some(true),
            speed_limit_bps: None,
            connections: 1,
            error_kind: None,
            error_message: None,
            error_detail: None,
            retry_count: 0,
            engine_options: EngineOptions::default(),
            peers: None,
            seeds: None,
            info_hash: None,
            title: None,
            thumbnail: None,
            created_at: t.clone(),
            started_at: Some(t.clone()),
            completed_at: None,
            updated_at: t,
            scheduled_at: None,
            file_missing: false,
        };
        db.insert_download(&d).unwrap();
        db.set_validators("crashed", &pixidl_core::db::Validators { etag: Some("\"v1\"".into()), last_modified: None }).unwrap();
        std::fs::write(dir.path().join("crash.bin.part"), &srv.state.data[..100_000]).unwrap();
        id = d.id;
    }
    let db = Db::open(&db_path).unwrap();
    let h = harness_from(db, dir, data_dir).await;
    h.wait_status(&id, DownloadStatus::Completed, 20).await;
    let bytes = std::fs::read(h.dir.path().join("crash.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data));
    assert_eq!(srv.ranges()[0].as_deref(), Some("bytes=100000-"), "resumed from verified on-disk size");
    assert!(h.mgr.events(&id).unwrap().iter().any(|e| e.kind == "recovered"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn graceful_shutdown_requeues_and_restart_continues() {
    let srv = start_server(1024 * 1024, 15).await;
    let dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let db_path = data_dir.path().join("pixidl.db");
    let db = Db::open(&db_path).unwrap();
    let mut s = test_settings(dir.path());
    s.connections_per_download = 1;
    db.save_settings(&s).unwrap();
    let h = harness_from(db, dir, data_dir).await;
    let d = h.mgr.add(req(srv.url("/slow/shut.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.downloaded_bytes > 100 * 1024).await;
    h.mgr.shutdown().await;
    let after = h.mgr.db().get_download(&d.id).unwrap().unwrap();
    assert_eq!(after.status, DownloadStatus::Queued);
    assert!(after.downloaded_bytes > 0);
    let Harness { dir, data_dir, mgr, .. } = h;
    drop(mgr);
    let h2 = harness_from(Db::open(&db_path).unwrap(), dir, data_dir).await;
    h2.wait_status(&d.id, DownloadStatus::Completed, 30).await;
    let bytes = std::fs::read(h2.dir.path().join("shut.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.state.data));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_download_waits() {
    let srv = start_server(1024, 0).await;
    let h = harness_with(|_| {}).await;
    let at = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
    let t = Instant::now();
    let d = h.mgr.add(AddDownloadRequest { url: srv.url("/file/s.bin"), scheduled_at: Some(at), ..Default::default() }, AddSource::User).await.unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(h.mgr.get(&d.id).unwrap().unwrap().status, DownloadStatus::Queued);
    h.wait_status(&d.id, DownloadStatus::Completed, 10).await;
    assert!(t.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disk_space_is_checked() {
    // Pretend the server announces an absurd size; the download must fail
    // with DiskFull instead of filling the disk.
    use axum::routing::get;
    let app = axum::Router::new().route(
        "/huge",
        get(|| async {
            axum::http::Response::builder()
                .status(206)
                .header("content-range", "bytes 0-1/900000000000000")
                .header("content-length", "2")
                .body(axum::body::Body::from("ab"))
                .unwrap()
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let h = harness_with(|_| {}).await;
    let d = h.mgr.add(req(format!("http://{addr}/huge")), AddSource::User).await.unwrap();
    let f = h.wait_status(&d.id, DownloadStatus::Failed, 10).await;
    assert_eq!(f.error_kind, Some(ErrorKind::DiskFull));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inspect_http_url() {
    let srv = start_server(4096, 0).await;
    let h = harness_with(|_| {}).await;
    let i = h.mgr.inspect_url(&srv.url("/cd"), None).await.unwrap();
    assert_eq!(i.engine, EngineKind::Http);
    assert_eq!(i.filename.as_deref(), Some("report final.pdf"));
    assert_eq!(i.total_bytes, Some(4096));
    assert_eq!(i.resumable, Some(true));
    assert_eq!(i.category, "Documents");
    let i = h.mgr.inspect_url(&srv.url("/missing/x"), None).await.unwrap();
    assert_eq!(i.warning.unwrap().kind, ErrorKind::NotFound);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pausing_never_reports_queue_finished() {
    // Regression: pausing the only download must not emit QueueFinished,
    // which can trigger the user's after-queue shutdown.
    let srv = start_server(1024 * 1024, 20).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let d = h.mgr.add(req(srv.url("/slow/p.bin")), AddSource::User).await.unwrap();
    h.wait_for(&d.id, 10, |d| d.downloaded_bytes > 64 * 1024).await;
    h.mgr.pause(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Paused, 10).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!h.events.lock().iter().any(|e| matches!(e, ManagerEvent::QueueFinished)));
    let e = h.mgr.add(req(srv.url("/missing/x.bin")), AddSource::User).await.unwrap();
    h.wait_status(&e.id, DownloadStatus::Failed, 10).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!h.events.lock().iter().any(|e| matches!(e, ManagerEvent::QueueFinished)), "a failure is not a finished queue either");
    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(h.events.lock().iter().filter(|e| matches!(e, ManagerEvent::QueueFinished)).count(), 1);
}
