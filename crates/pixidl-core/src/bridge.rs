//! Local bridge between the native-messaging host and the running app.
//!
//! - TCP bound to 127.0.0.1 only (never reachable from the network)
//! - a fresh 256-bit random token per app launch, written with the port to
//!   `bridge.json` in the user's private data folder; the first frame of every
//!   connection must present it (constant-time comparison)
//! - frames are length-prefixed (u32 LE) JSON, capped at 1 MiB
//! - bounded concurrency and idle timeouts
//!
//! A web page cannot speak this protocol: it cannot read the token, and an
//! HTTP request to the port fails authentication immediately.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::manager::DownloadManager;
use crate::protocol::{self, framing, ErrorCode, ProtocolError};

const MAX_CONNECTIONS: usize = 16;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub app_version: String,
    pub protocol_version: u32,
}

#[derive(Serialize, Deserialize)]
struct Auth {
    auth: String,
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // On Windows the per-user AppData folder is already restricted to the user.
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub struct BridgeServer {
    pub addr: SocketAddr,
    info_path: std::path::PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl BridgeServer {
    /// Starts listening and publishes `bridge.json`.
    pub async fn start(mgr: DownloadManager, info_path: &Path) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token = hex::encode(raw);
        let info = BridgeInfo {
            port: addr.port(),
            token: token.clone(),
            pid: std::process::id(),
            app_version: crate::APP_VERSION.into(),
            protocol_version: protocol::PROTOCOL_VERSION,
        };
        write_private(info_path, serde_json::to_vec_pretty(&info).unwrap().as_slice())?;
        let token = Arc::new(token);
        let sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else { continue };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let Ok(permit) = sem.clone().try_acquire_owned() else {
                    tracing::warn!("bridge: too many connections");
                    continue;
                };
                let mgr = mgr.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_conn(stream, mgr, &token).await {
                        tracing::debug!(error = %e, "bridge connection ended");
                    }
                    drop(permit);
                });
            }
        });
        tracing::info!(port = addr.port(), "browser bridge listening on loopback");
        Ok(Self { addr, info_path: info_path.to_path_buf(), task })
    }

    pub fn stop(&self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.info_path);
    }
}

impl Drop for BridgeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn serve_conn(mut s: TcpStream, mgr: DownloadManager, token: &str) -> std::io::Result<()> {
    s.set_nodelay(true)?;
    let first = tokio::time::timeout(Duration::from_secs(5), framing::read_frame_async(&mut s))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "auth timeout"))??;
    let authed = first
        .and_then(|f| serde_json::from_slice::<Auth>(&f).ok())
        .is_some_and(|a| constant_time_eq(a.auth.as_bytes(), token.as_bytes()));
    if !authed {
        let resp = protocol::error_response(None, &ProtocolError { code: ErrorCode::Unauthorized, message: "Unauthorized".into() });
        let _ = framing::write_frame_async(&mut s, resp.to_string().as_bytes()).await;
        tracing::warn!("bridge: rejected unauthenticated connection");
        return Ok(());
    }
    framing::write_frame_async(&mut s, br#"{"authenticated":true}"#).await?;
    loop {
        let frame = match tokio::time::timeout(IDLE_TIMEOUT, framing::read_frame_async(&mut s)).await {
            Err(_) => return Ok(()),
            Ok(r) => r?,
        };
        let Some(frame) = frame else { return Ok(()) };
        let resp = protocol::handle(&mgr, &frame).await;
        framing::write_frame_async(&mut s, resp.to_string().as_bytes()).await?;
    }
}

/// Client used by the native host.
pub struct BridgeClient {
    stream: TcpStream,
}

impl BridgeClient {
    pub async fn connect(info: &BridgeInfo) -> std::io::Result<Self> {
        let mut stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(("127.0.0.1", info.port)))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout"))??;
        stream.set_nodelay(true)?;
        let auth = serde_json::to_vec(&Auth { auth: info.token.clone() }).unwrap();
        framing::write_frame_async(&mut stream, &auth).await?;
        let reply = tokio::time::timeout(Duration::from_secs(5), framing::read_frame_async(&mut stream))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "auth timeout"))??
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "closed"))?;
        let v: serde_json::Value = serde_json::from_slice(&reply).unwrap_or_default();
        if v.get("authenticated") != Some(&serde_json::Value::Bool(true)) {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "bridge rejected token"));
        }
        Ok(Self { stream })
    }

    pub async fn request(&mut self, raw: &[u8]) -> std::io::Result<Vec<u8>> {
        framing::write_frame_async(&mut self.stream, raw).await?;
        tokio::time::timeout(Duration::from_secs(60), framing::read_frame_async(&mut self.stream))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "response timeout"))??
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "closed"))
    }
}

pub fn read_info(path: &Path) -> Option<BridgeInfo> {
    let data = std::fs::read(path).ok()?;
    if data.len() > 4096 {
        return None;
    }
    serde_json::from_slice(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
