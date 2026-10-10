//! User-created queues, bulk (selection) actions and "clear completed",
//! against a real local HTTP server.

mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::*;
use pixidl_core::manager::AddSource;
use pixidl_core::types::*;

fn req_in(url: String, queue: &str) -> AddDownloadRequest {
    AddDownloadRequest { url, queue_id: Some(queue.to_string()), ..Default::default() }
}

fn paused(url: String) -> AddDownloadRequest {
    AddDownloadRequest { url, start_paused: true, ..Default::default() }
}

/// Polls until every download completes; returns the most downloads seen
/// running at once, overall and per queue.
async fn run_to_completion(h: &Harness, timeout_s: u64) -> (usize, HashMap<String, usize>) {
    let start = Instant::now();
    let mut max_total = 0;
    let mut max_per_queue: HashMap<String, usize> = HashMap::new();
    loop {
        let list = h.mgr.list().unwrap();
        let running: Vec<&Download> = list.iter().filter(|d| d.status.is_running()).collect();
        max_total = max_total.max(running.len());
        let mut per: HashMap<String, usize> = HashMap::new();
        for d in &running {
            *per.entry(d.queue_id.clone()).or_default() += 1;
        }
        for (q, n) in per {
            let m = max_per_queue.entry(q).or_default();
            *m = (*m).max(n);
        }
        if list.iter().all(|d| d.status == DownloadStatus::Completed) {
            return (max_total, max_per_queue);
        }
        assert!(start.elapsed() < Duration::from_secs(timeout_s), "timeout: {:?}", list.iter().map(|d| (d.queue_id.clone(), d.status)).collect::<Vec<_>>());
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_queue_limit_and_the_global_limit_apply() {
    let srv = start_server(256 * 1024, 15).await;
    let h = harness_with(|s| {
        s.max_concurrent_downloads = 3;
        s.connections_per_download = 1;
    })
    .await;
    let a = h.mgr.create_queue("Queue A", 1).unwrap();
    let b = h.mgr.create_queue("Queue B", 1).unwrap();
    for i in 0..3 {
        h.mgr.add(req_in(srv.url(&format!("/slow/a{i}.bin")), &a.id), AddSource::User).await.unwrap();
        h.mgr.add(req_in(srv.url(&format!("/slow/b{i}.bin")), &b.id), AddSource::User).await.unwrap();
    }
    let (max_total, per_queue) = run_to_completion(&h, 60).await;
    assert_eq!(max_total, 2, "two queues limited to 1 each: exactly 2 run although the global limit is 3");
    assert_eq!(per_queue.get(&a.id), Some(&1));
    assert_eq!(per_queue.get(&b.id), Some(&1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_limit_still_caps_all_queues() {
    let srv = start_server(256 * 1024, 15).await;
    let h = harness_with(|s| {
        s.max_concurrent_downloads = 1;
        s.connections_per_download = 1;
    })
    .await;
    let a = h.mgr.create_queue("Queue A", 5).unwrap();
    for i in 0..2 {
        h.mgr.add(req_in(srv.url(&format!("/slow/a{i}.bin")), &a.id), AddSource::User).await.unwrap();
        h.mgr.add(req_in(srv.url(&format!("/slow/m{i}.bin")), MAIN_QUEUE_ID), AddSource::User).await.unwrap();
    }
    let (max_total, _) = run_to_completion(&h, 60).await;
    assert_eq!(max_total, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_crud_and_validation() {
    let h = harness_with(|_| {}).await;
    let queues = h.mgr.queues().unwrap();
    assert_eq!(queues.len(), 1);
    assert!(queues[0].is_main());
    let q = h.mgr.create_queue("  Night  ", 50).unwrap();
    assert_eq!(q.name, "Night");
    assert_eq!(q.max_concurrent, 20, "clamped to 1–20");
    assert!(h.mgr.create_queue("night", 1).is_err(), "names are unique (case-insensitive)");
    assert!(h.mgr.create_queue("   ", 1).is_err());
    h.mgr.rename_queue(&q.id, "Overnight").unwrap();
    h.mgr.set_queue_max_concurrent(&q.id, 0).unwrap();
    let q2 = h.mgr.queues().unwrap().into_iter().find(|x| x.id == q.id).unwrap();
    assert_eq!((q2.name.as_str(), q2.max_concurrent), ("Overnight", 1));
    // The main queue can be renamed (and reset), never deleted.
    h.mgr.rename_queue(MAIN_QUEUE_ID, "Everything").unwrap();
    h.mgr.rename_queue(MAIN_QUEUE_ID, "").unwrap();
    assert_eq!(h.mgr.queues().unwrap()[0].name, "");
    assert!(h.mgr.delete_queue(MAIN_QUEUE_ID).is_err());
    assert!(h.events.lock().iter().any(|e| matches!(e, ManagerEvent::QueuesChanged { .. })));
    // Unknown queues are rejected when adding.
    let err = h.mgr.add(req_in("https://example.com/x.zip".into(), "nope"), AddSource::User).await;
    assert!(err.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_cannot_choose_a_queue() {
    let h = harness_with(|_| {}).await;
    let q = h.mgr.create_queue("Night", 1).unwrap();
    h.mgr.stop_queue(&q.id).unwrap();
    let d = h.mgr.add(AddDownloadRequest { start_paused: true, ..req_in("https://example.com/b.zip".into(), &q.id) }, AddSource::Browser).await.unwrap();
    assert_eq!(d.queue_id, MAIN_QUEUE_ID);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn move_to_queue_and_delete_queue() {
    let srv = start_server(64 * 1024, 0).await;
    let h = harness_with(|_| {}).await;
    let q = h.mgr.create_queue("Later", 2).unwrap();
    let a = h.mgr.add(paused(srv.url("/file/a.bin")), AddSource::User).await.unwrap();
    let b = h.mgr.add(paused(srv.url("/file/b.bin")), AddSource::User).await.unwrap();
    assert_eq!(a.queue_id, MAIN_QUEUE_ID);
    h.mgr.move_to_queue(&[a.id.clone(), b.id.clone(), "gone".into()], &q.id).unwrap();
    for id in [&a.id, &b.id] {
        assert_eq!(h.mgr.get(id).unwrap().unwrap().queue_id, q.id);
    }
    assert!(h.events.lock().iter().any(|e| matches!(e, ManagerEvent::DownloadUpdated { download } if download.id == a.id && download.queue_id == q.id)));
    assert!(h.mgr.move_to_queue(std::slice::from_ref(&a.id), "no-such-queue").is_err());

    h.mgr.delete_queue(&q.id).unwrap();
    assert!(h.mgr.queues().unwrap().iter().all(|x| x.id != q.id));
    for id in [&a.id, &b.id] {
        assert_eq!(h.mgr.get(id).unwrap().unwrap().queue_id, MAIN_QUEUE_ID, "downloads of a deleted queue move to main");
    }
    // They still download normally from the main queue.
    h.mgr.resume_many(&[a.id.clone(), b.id.clone()]).unwrap();
    h.wait_status(&a.id, DownloadStatus::Completed, 20).await;
    h.wait_status(&b.id, DownloadStatus::Completed, 20).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_queue_starts_nothing_until_started() {
    let srv = start_server(512 * 1024, 15).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let q = h.mgr.create_queue("Night", 2).unwrap();
    h.mgr.stop_queue(&q.id).unwrap();
    let a = h.mgr.add(req_in(srv.url("/slow/a.bin"), &q.id), AddSource::User).await.unwrap();
    let b = h.mgr.add(AddDownloadRequest { start_paused: true, ..req_in(srv.url("/slow/b.bin"), &q.id) }, AddSource::User).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(h.mgr.get(&a.id).unwrap().unwrap().status, DownloadStatus::Queued, "a stopped queue starts nothing");
    // Starting the queue starts its queued downloads and resumes its paused ones.
    h.mgr.start_queue(&q.id).unwrap();
    h.wait_for(&b.id, 10, |d| d.status.is_running()).await;
    h.wait_for(&a.id, 10, |d| d.status.is_running()).await;
    // Stopping it sends running downloads back to waiting.
    h.mgr.stop_queue(&q.id).unwrap();
    h.wait_status(&a.id, DownloadStatus::Queued, 10).await;
    h.wait_status(&b.id, DownloadStatus::Queued, 10).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(h.mgr.get(&a.id).unwrap().unwrap().status, DownloadStatus::Queued);
    h.mgr.start_queue(&q.id).unwrap();
    h.wait_status(&a.id, DownloadStatus::Completed, 30).await;
    h.wait_status(&b.id, DownloadStatus::Completed, 30).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_many_and_resume_many() {
    let srv = start_server(1024 * 1024, 20).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let a = h.mgr.add(paused(srv.url("/slow/a.bin")), AddSource::User).await.unwrap();
    let b = h.mgr.add(paused(srv.url("/slow/b.bin")), AddSource::User).await.unwrap();
    let ids = vec![a.id.clone(), b.id.clone(), "missing".to_string()];
    h.mgr.resume_many(&ids).unwrap();
    h.wait_for(&a.id, 10, |d| d.status == DownloadStatus::Downloading).await;
    h.wait_for(&b.id, 10, |d| d.status == DownloadStatus::Downloading).await;
    h.mgr.pause_many(&ids).unwrap();
    h.wait_status(&a.id, DownloadStatus::Paused, 10).await;
    h.wait_status(&b.id, DownloadStatus::Paused, 10).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_many_handles_running_and_finished_downloads() {
    let srv = start_server(2 * 1024 * 1024, 20).await;
    let h = harness_with(|s| s.connections_per_download = 1).await;
    let done = h.mgr.add(AddDownloadRequest { url: srv.url("/file/done.bin"), ..Default::default() }, AddSource::User).await.unwrap();
    h.wait_status(&done.id, DownloadStatus::Completed, 10).await;
    let running = h.mgr.add(AddDownloadRequest { url: srv.url("/slow/run.bin"), ..Default::default() }, AddSource::User).await.unwrap();
    h.wait_for(&running.id, 10, |d| d.downloaded_bytes > 32 * 1024).await;
    let keep = h.mgr.add(paused(srv.url("/file/keep.bin")), AddSource::User).await.unwrap();

    h.mgr.remove_many(&[done.id.clone(), running.id.clone(), "missing".into()], false).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.mgr.get(&running.id).unwrap().is_some() {
        assert!(Instant::now() < deadline, "running download was not removed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(h.mgr.get(&done.id).unwrap().is_none());
    assert!(h.mgr.get(&keep.id).unwrap().is_some(), "unselected downloads stay");
    assert!(h.dir.path().join("done.bin").exists(), "completed file kept without delete_files");
    assert!(!h.dir.path().join("run.bin.part").exists(), "partial data is always removed");
    let removed: Vec<String> = h.events.lock().iter().filter_map(|e| if let ManagerEvent::DownloadRemoved { id } = e { Some(id.clone()) } else { None }).collect();
    assert!(removed.contains(&done.id) && removed.contains(&running.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clear_completed_removes_only_completed_and_keeps_files() {
    let srv = start_server(32 * 1024, 0).await;
    let h = harness_with(|_| {}).await;
    let a = h.mgr.add(AddDownloadRequest { url: srv.url("/file/a.bin"), ..Default::default() }, AddSource::User).await.unwrap();
    let b = h.mgr.add(AddDownloadRequest { url: srv.url("/file/b.bin"), ..Default::default() }, AddSource::User).await.unwrap();
    h.wait_status(&a.id, DownloadStatus::Completed, 10).await;
    h.wait_status(&b.id, DownloadStatus::Completed, 10).await;
    let failed = h.mgr.add(AddDownloadRequest { url: srv.url("/missing/x.bin"), ..Default::default() }, AddSource::User).await.unwrap();
    h.wait_status(&failed.id, DownloadStatus::Failed, 10).await;
    let waiting = h.mgr.add(paused(srv.url("/file/c.bin")), AddSource::User).await.unwrap();

    assert_eq!(h.mgr.clear_completed(false).await.unwrap(), 2);
    let left: Vec<String> = h.mgr.list().unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(left.len(), 2);
    assert!(left.contains(&failed.id) && left.contains(&waiting.id));
    assert!(h.dir.path().join("a.bin").exists() && h.dir.path().join("b.bin").exists(), "files stay on disk");
    assert_eq!(h.mgr.clear_completed(false).await.unwrap(), 0);
}
