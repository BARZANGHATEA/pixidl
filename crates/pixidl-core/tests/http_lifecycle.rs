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
    // The first response serves segment 0; the other segments made their own ranged requests.
    assert_eq!(srv.ranges()[0].as_deref(), Some("bytes=0-"));
    let ranged = srv.ranges().iter().filter(|r| r.as_deref().is_some_and(|r| r.contains('-') && !r.ends_with('-'))).count();
    assert!(ranged >= 3, "expected segment requests, got {:?}", srv.ranges());
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
            queue_id: MAIN_QUEUE_ID.into(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checksum_is_verified() {
    let srv = start_server(200 * 1024, 0).await;
    let h = harness_with(|_| {}).await;
    let good = hex::encode(Sha256::digest(&*srv.state.data));
    let ok = h
        .mgr
        .add(AddDownloadRequest { url: srv.url("/file/good.bin"), engine_options: EngineOptions { sha256: Some(good.to_uppercase()), ..Default::default() }, ..Default::default() }, AddSource::User)
        .await
        .unwrap();
    h.wait_status(&ok.id, DownloadStatus::Completed, 20).await;
    assert!(h.mgr.events(&ok.id).unwrap().iter().any(|e| e.kind == "checksum_verified"));

    let bad = h
        .mgr
        .add(AddDownloadRequest { url: srv.url("/file/bad.bin"), engine_options: EngineOptions { sha256: Some("0".repeat(64)), ..Default::default() }, ..Default::default() }, AddSource::User)
        .await
        .unwrap();
    let f = h.wait_status(&bad.id, DownloadStatus::Failed, 20).await;
    assert_eq!(f.error_kind, Some(ErrorKind::ChecksumMismatch));
    assert!(h.dir.path().join("bad.bin").exists(), "file kept for inspection");

    let invalid = h.mgr.add(AddDownloadRequest { url: srv.url("/file/x.bin"), engine_options: EngineOptions { sha256: Some("abc".into()), ..Default::default() }, ..Default::default() }, AddSource::User).await;
    assert!(invalid.is_err());
}

// ------------------------------------------------------- segmented downloads

const MIB: u64 = 1024 * 1024;

/// Whether `offset` lies in a byte range the segments say is not on disk yet.
fn in_missing_range(segs: &[pixidl_core::db::Segment], offset: u64) -> bool {
    segs.iter().any(|s| offset >= s.start + s.downloaded && offset <= s.end_incl)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_connections_steal_work_until_the_end() {
    // 16 MiB over 4 connections at 2 MiB/s each, but the range starting at 0
    // is 4x slower. A static split would leave one slow connection running
    // alone for the last ~6 s (8 s total); with work stealing the three idle
    // connections take over halves of the slow range.
    let srv = start_range_server(RangeServerOpts { size: 16 * MIB as usize, bytes_per_sec: 2 * MIB, slow_zero_factor: 4, conn_limit: 0 }).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let t = Instant::now();
    let d = h.mgr.add(req(srv.url("/r/steal.bin")), AddSource::User).await.unwrap();
    let total = 16 * MIB;
    let (mut late_inflight, mut max_segments, mut max_view_conns) = (0, 0, 0);
    loop {
        let cur = h.mgr.get(&d.id).unwrap().unwrap();
        if cur.status == DownloadStatus::Completed {
            break;
        }
        assert!(!matches!(cur.status, DownloadStatus::Failed), "{:?}", cur.error_message);
        assert!(t.elapsed() < Duration::from_secs(30), "timeout");
        if let Some(v) = h.mgr.segments(&d.id).unwrap() {
            assert_eq!(v.total, total);
            assert_eq!(v.segments.first().unwrap().start, 0);
            assert_eq!(v.segments.last().unwrap().end, total);
            assert!(v.segments.windows(2).all(|w| w[0].end == w[1].start), "segments must tile the file: {v:?}");
            max_segments = max_segments.max(v.segments.len());
            max_view_conns = max_view_conns.max(v.connections);
        }
        // Once the fast ranges are done and stealing has started, the idle
        // connections must be working again (a static split drops to one).
        if srv.requests().len() > 4 {
            late_inflight = late_inflight.max(srv.inflight());
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let elapsed = t.elapsed();
    let bytes = std::fs::read(h.dir.path().join("steal.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.data));
    let reqs = srv.requests();
    // All four connections ran at the same time …
    assert!(srv.max_inflight.load(std::sync::atomic::Ordering::SeqCst) >= 4, "{reqs:?}");
    assert!(max_view_conns >= 4, "segment view reported {max_view_conns} connections");
    // … the first response was reused for the first range (no extra request) …
    assert_eq!(reqs[0], (0, total - 1));
    assert_eq!(reqs.iter().filter(|r| r.0 == 0).count(), 1, "{reqs:?}");
    // … idle connections split ranges at new offsets …
    let planned = [0, 4 * MIB, 8 * MIB, 12 * MIB];
    let steals: Vec<_> = reqs.iter().filter(|r| !planned.contains(&r.0)).collect();
    assert!(!steals.is_empty() && reqs.len() > 4, "no work was stolen: {reqs:?}");
    assert!(max_segments > 4);
    // … so several connections were busy again after the first ranges finished …
    assert!(late_inflight >= 3, "only {late_inflight} connections active after stealing began");
    // … and the slow range did not dominate the total time.
    assert!(elapsed < Duration::from_millis(6500), "took {elapsed:?}; a static split needs ≥ 8 s");
    assert!(h.mgr.db().load_segments(&d.id).unwrap().is_empty(), "segments cleared after completion");
    assert!(h.mgr.segments(&d.id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_limit_reduces_connections_instead_of_failing() {
    // The server answers 503 to a third concurrent connection.
    let srv = start_range_server(RangeServerOpts { size: 8 * MIB as usize, bytes_per_sec: 2 * MIB, slow_zero_factor: 1, conn_limit: 2 }).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(req(srv.url("/r/limited.bin")), AddSource::User).await.unwrap();
    let done = h.wait_for(&d.id, 30, |d| matches!(d.status, DownloadStatus::Completed | DownloadStatus::Failed)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?} {:?}", done.error_message, done.error_detail);
    assert_eq!(done.retry_count, 0, "no download-level retry was needed");
    let bytes = std::fs::read(h.dir.path().join("limited.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.data));
    assert!(srv.rejected.load(std::sync::atomic::Ordering::SeqCst) >= 1, "the limit was hit");
    assert_eq!(srv.max_inflight.load(std::sync::atomic::Ordering::SeqCst), 2, "two connections ran in parallel");
    assert!(done.connections < 4, "connection count reduced, got {}", done.connections);
    let events = h.mgr.events(&d.id).unwrap();
    let limit_events = events.iter().filter(|e| e.message.as_deref().unwrap_or("").contains("Server allows only")).count();
    assert_eq!(limit_events, 1, "{events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_after_splits_keeps_file_intact() {
    let srv = start_range_server(RangeServerOpts { size: 16 * MIB as usize, bytes_per_sec: 2 * MIB, slow_zero_factor: 4, conn_limit: 0 }).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(req(srv.url("/r/split-pause.bin")), AddSource::User).await.unwrap();
    // Wait until idle connections have split ranges, then a little longer.
    let t = Instant::now();
    while h.mgr.segments(&d.id).unwrap().map_or(0, |v| v.segments.len()) <= 5 {
        assert!(t.elapsed() < Duration::from_secs(20), "no split happened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.mgr.pause(&d.id).unwrap();
    let paused = h.wait_status(&d.id, DownloadStatus::Paused, 10).await;
    let mut segs = h.mgr.db().load_segments(&d.id).unwrap();
    segs.sort_by_key(|s| s.start);
    assert!(segs.len() > 5, "{segs:?}");
    assert!(pixidl_core::engines::http::segments_cover(&segs, 16 * MIB), "{segs:?}");
    let on_disk: u64 = segs.iter().map(|s| s.downloaded).sum();
    assert!(on_disk > 0 && on_disk < 16 * MIB && on_disk <= paused.downloaded_bytes, "{on_disk} vs {}", paused.downloaded_bytes);
    let view = h.mgr.segments(&d.id).unwrap().expect("paused download keeps its segment map");
    assert_eq!(view.connections, 0);
    assert!(view.segments.iter().all(|s| !s.active));
    let before = srv.requests().len();

    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 30).await;
    let bytes = std::fs::read(h.dir.path().join("split-pause.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.data));
    // After resuming, only bytes that were not on disk were requested.
    let after = &srv.requests()[before..];
    assert!(!after.is_empty());
    for r in after {
        assert!(in_missing_range(&segs, r.0), "re-requested committed data at {}: {segs:?}", r.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_with_holes_refetches_only_uncommitted_bytes() {
    // Persisted state as after a crash: some ranges complete, some partial,
    // and garbage in the partial file wherever nothing was committed.
    use pixidl_core::db::{Segment, Validators};
    let total = 4 * MIB;
    let srv = start_range_server(RangeServerOpts { size: total as usize, ..Default::default() }).await;
    let h = harness_with(|s| s.connections_per_download = 4).await;
    let d = h.mgr.add(AddDownloadRequest { url: srv.url("/r/holes.bin"), start_paused: true, ..Default::default() }, AddSource::User).await.unwrap();
    let segs = vec![
        Segment { idx: 0, start: 0, end_incl: MIB - 1, downloaded: MIB },
        Segment { idx: 1, start: MIB, end_incl: 2 * MIB - 1, downloaded: 300_000 },
        Segment { idx: 5, start: 2 * MIB, end_incl: 3 * MIB - 1, downloaded: 0 },
        Segment { idx: 2, start: 3 * MIB, end_incl: 4 * MIB - 1, downloaded: MIB },
    ];
    let mut part = vec![0xAAu8; total as usize];
    for s in &segs {
        let (a, b) = (s.start as usize, (s.start + s.downloaded) as usize);
        part[a..b].copy_from_slice(&srv.data[a..b]);
    }
    std::fs::write(h.dir.path().join("holes.bin.part"), &part).unwrap();
    h.mgr.db().save_segments(&d.id, &segs).unwrap();
    h.mgr.db().set_validators(&d.id, &Validators { etag: Some("\"r1\"".into()), last_modified: None }).unwrap();

    h.mgr.resume(&d.id).unwrap();
    h.wait_status(&d.id, DownloadStatus::Completed, 20).await;
    let bytes = std::fs::read(h.dir.path().join("holes.bin")).unwrap();
    assert_eq!(sha(&bytes), sha(&srv.data));
    let reqs = srv.requests();
    // The first request starts at the first missing byte and is reused for that range.
    assert_eq!(reqs[0], (MIB + 300_000, total - 1));
    for r in &reqs {
        assert!(in_missing_range(&segs, r.0), "requested committed data at {}: {reqs:?}", r.0);
    }
}

/// Throughput comparison; run with
/// `cargo test -p pixidl-core --test http_lifecycle -- --ignored --nocapture benchmark`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: takes about a minute"]
async fn benchmark_one_vs_eight_connections() {
    let srv = start_range_server(RangeServerOpts { size: 50 * MIB as usize, bytes_per_sec: MIB, slow_zero_factor: 1, conn_limit: 0 }).await;
    for conns in [1u32, 8] {
        let h = harness_with(|s| s.connections_per_download = conns).await;
        let t = Instant::now();
        let d = h.mgr.add(req(srv.url(&format!("/r/bench{conns}.bin"))), AddSource::User).await.unwrap();
        h.wait_status(&d.id, DownloadStatus::Completed, 180).await;
        let e = t.elapsed();
        let bytes = std::fs::read(h.dir.path().join(format!("bench{conns}.bin"))).unwrap();
        assert_eq!(sha(&bytes), sha(&srv.data));
        println!("50 MiB at 1 MiB/s per connection, {conns} connection(s): {:.2} s ({:.2} MiB/s)", e.as_secs_f64(), 50.0 / e.as_secs_f64());
    }
}
