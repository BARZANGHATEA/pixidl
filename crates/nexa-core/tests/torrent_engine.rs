//! Torrent engine test, fully offline: a librqbit seeder on localhost, a
//! minimal HTTP tracker that points to it, and the manager as the leecher.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine as _;
use common::*;
use librqbit::{AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session, SessionOptions};
use nexa_core::manager::AddSource;
use nexa_core::types::*;

/// Serves `/announce` with a compact peer list containing only `peer`.
async fn start_tracker(peer: SocketAddr) -> String {
    let app = axum::Router::new().route(
        "/announce",
        axum::routing::get(move || async move {
            let ip = match peer.ip() {
                std::net::IpAddr::V4(v4) => v4.octets(),
                _ => [127, 0, 0, 1],
            };
            let mut body = b"d8:intervali5e5:peers6:".to_vec();
            body.extend_from_slice(&ip);
            body.extend_from_slice(&peer.port().to_be_bytes());
            body.push(b'e');
            body
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/announce")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn torrent_file_downloads_from_local_seeder_with_pause_resume() {
    // Content to share: a folder with two files (multi-file torrent).
    let seed_root = tempfile::tempdir().unwrap();
    let content = seed_root.path().join("collection");
    std::fs::create_dir_all(&content).unwrap();
    let a = payload(3 * 1024 * 1024);
    let b = payload(700 * 1024);
    std::fs::write(content.join("a.bin"), &a).unwrap();
    std::fs::write(content.join("b.bin"), &b).unwrap();

    // Seeder session listening on localhost.
    let seed_state = tempfile::tempdir().unwrap();
    let seeder = Session::new_with_opts(
        seed_root.path().to_path_buf(),
        SessionOptions {
            dht: None,
            persistence: None,
            listen: Some(ListenerOptions { listen_addr: "127.0.0.1:0".parse().unwrap(), ipv4_only: true, ..Default::default() }),
            disable_local_service_discovery: true,
            ipv4_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seed_addr = seeder.listen_addr().expect("seeder listening");
    let tracker = start_tracker(SocketAddr::from(([127, 0, 0, 1], seed_addr.port()))).await;
    let created = librqbit::create_torrent(
        &content,
        CreateTorrentOptions { name: Some("collection"), trackers: vec![tracker.clone()], piece_length: Some(64 * 1024) },
        &librqbit::spawn_utils::BlockingSpawner::new(4),
    )
    .await
    .unwrap();
    let torrent_bytes = created.as_bytes().unwrap().to_vec();
    seeder
        .add_torrent(
            AddTorrent::from_bytes(torrent_bytes.clone()),
            Some(AddTorrentOptions { output_folder: Some(content.to_string_lossy().into()), overwrite: true, ..Default::default() }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap()
        .wait_until_initialized()
        .await
        .unwrap();
    drop(seed_state);

    // Leecher: the real manager, DHT off so the test never leaves localhost.
    let h = harness_with(|s| {
        s.torrent_enable_dht = false;
        s.global_speed_limit_bps = Some(1024 * 1024); // slow enough to pause mid-way
    })
    .await;

    // Inspect gives the real file list without downloading.
    let info = h.mgr.inspect_torrent_file(torrent_bytes.clone()).await.unwrap();
    assert_eq!(info.name, "collection");
    assert_eq!(info.files.len(), 2);
    assert_eq!(info.total_bytes, (a.len() + b.len()) as u64);

    let d = h
        .mgr
        .add(
            AddDownloadRequest { url: String::new(), torrent_base64: Some(base64::engine::general_purpose::STANDARD.encode(&torrent_bytes)), ..Default::default() },
            AddSource::User,
        )
        .await
        .unwrap();
    assert_eq!(d.engine, EngineKind::Torrent);
    assert_eq!(d.category, "Torrents");

    let mid = h.wait_for(&d.id, 60, |d| d.status == DownloadStatus::Downloading && d.downloaded_bytes > 256 * 1024).await;
    assert_eq!(mid.filename, "collection");
    assert!(mid.info_hash.is_some());
    let live = h.wait_for(&d.id, 10, |d| d.peers.is_some()).await;
    assert!(live.peers.unwrap() >= 1, "peer count comes from the engine");
    assert_eq!(mid.seeds, None, "seeds are never invented");
    assert_eq!(mid.total_bytes, Some(info.total_bytes));

    h.mgr.pause(&d.id).unwrap();
    let paused = h.wait_status(&d.id, DownloadStatus::Paused, 20).await;
    assert!(paused.downloaded_bytes > 0);

    // Lift the limit and resume.
    h.mgr.set_global_limit(None).unwrap();
    h.mgr.resume(&d.id).unwrap();
    let done = h.wait_status(&d.id, DownloadStatus::Completed, 120).await;
    assert_eq!(done.downloaded_bytes, info.total_bytes);
    let out = h.dir.path().join("collection");
    assert_eq!(std::fs::read(out.join("a.bin")).unwrap(), a);
    assert_eq!(std::fs::read(out.join("b.bin")).unwrap(), b);

    seeder.stop().await;
    h.mgr.shutdown().await;
    let _ = Duration::from_secs(0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_magnet_is_rejected() {
    let h = harness_with(|s| s.torrent_enable_dht = false).await;
    assert!(h.mgr.add(AddDownloadRequest { url: "magnet:?dn=no-hash".into(), ..Default::default() }, AddSource::User).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn magnet_without_any_peer_source_fails_clearly() {
    let h = harness_with(|s| {
        s.torrent_enable_dht = false;
        s.max_concurrent_downloads = 1;
    })
    .await;
    let d = h
        .mgr
        .add(
            AddDownloadRequest { url: "magnet:?xt=urn:btih:c9e15763f722f23e98a29decdfae341b98d53056&dn=Some%20Show".into(), ..Default::default() },
            AddSource::User,
        )
        .await
        .unwrap();
    assert_eq!(d.engine, EngineKind::Torrent);
    assert_eq!(d.filename, "Some Show");
    // Without DHT and without trackers in the magnet, metadata can never be
    // found: the engine reports that clearly instead of hanging.
    let f = h.wait_status(&d.id, DownloadStatus::Failed, 15).await;
    assert_eq!(f.error_kind, Some(ErrorKind::TorrentMetadataUnavailable));
    assert!(f.error_detail.unwrap().contains("no DHT"));
    // A failed download can be cancelled (and its partial data discarded).
    h.mgr.cancel(&d.id).await.unwrap();
    h.wait_status(&d.id, DownloadStatus::Cancelled, 10).await;
}
