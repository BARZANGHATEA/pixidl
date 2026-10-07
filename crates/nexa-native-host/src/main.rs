//! `nexa-native-host` — the native-messaging host for Nexa Download Manager.
//!
//! Started by the browser with stdio connected to the extension. Each message
//! is validated locally, then forwarded over the authenticated loopback bridge
//! to the running app (which is started in the background if needed).
//!
//! Maintenance commands (used by the installer and the app):
//!   --register [--extension-id ID]... [--firefox-id ID]...
//!   --unregister
//!   --status
//!   --version

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nexa_core::bridge::{read_info, BridgeClient, BridgeInfo};
use nexa_core::protocol::{self, framing, ErrorCode, ProtocolError};
use nexa_core::{browser, paths};

const LOG_LIMIT: u64 = 1024 * 1024;

fn log(msg: &str) {
    let dir = paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("native-host.log");
    if std::fs::metadata(&path).map(|m| m.len() > LOG_LIMIT).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("native-host.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} [{}] {}", nexa_core::db::now(), std::process::id(), msg);
    }
}

fn reply(out: &mut impl Write, v: &serde_json::Value) -> io::Result<()> {
    framing::write_frame(out, v.to_string().as_bytes())
}

fn app_candidates() -> Vec<PathBuf> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return vec![] };
    ["nexa-download-manager", "Nexa Download Manager"]
        .iter()
        .map(|n| dir.join(nexa_core::tools::exe_name(n)))
        .filter(|p| p.is_file())
        .collect()
}

fn launch_app() -> bool {
    for app in app_candidates() {
        let mut cmd = std::process::Command::new(&app);
        cmd.arg("--background").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
            // Break away from the browser's job object so closing the
            // extension port does not kill the app.
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
            if cmd.spawn().is_ok() {
                log("launched app (breakaway)");
                return true;
            }
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        match cmd.spawn() {
            Ok(_) => {
                log("launched app");
                return true;
            }
            Err(e) => log(&format!("could not launch app: {e}")),
        }
    }
    false
}

async fn try_connect() -> Option<BridgeClient> {
    let info: BridgeInfo = read_info(&paths::bridge_file())?;
    BridgeClient::connect(&info).await.ok()
}

async fn connect_or_launch() -> Option<BridgeClient> {
    if let Some(c) = try_connect().await {
        return Some(c);
    }
    if !launch_app() {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(c) = try_connect().await {
            return Some(c);
        }
    }
    None
}

fn unavailable(id: Option<&str>) -> serde_json::Value {
    protocol::error_response(id, &ProtocolError { code: ErrorCode::AppUnavailable, message: "Nexa Download Manager is not running and could not be started".into() })
}

async fn run_host() -> i32 {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut client: Option<BridgeClient> = None;
    loop {
        let frame = match framing::read_frame(&mut stdin) {
            Ok(Some(f)) => f,
            Ok(None) => return 0, // browser closed the port
            Err(e) => {
                log(&format!("invalid frame from browser: {e}"));
                let _ = reply(&mut stdout, &protocol::error_response(None, &ProtocolError { code: ErrorCode::MessageTooLarge, message: "Invalid or oversized message".into() }));
                return 1;
            }
        };
        // Validate locally: malformed input never reaches the app.
        let env = match protocol::parse(&frame) {
            Ok(e) => e,
            Err((id, e)) => {
                log(&format!("rejected message: {:?}", e.code));
                if reply(&mut stdout, &protocol::error_response(id.as_deref(), &e)).is_err() {
                    return 1;
                }
                continue;
            }
        };
        let mut response = None;
        for attempt in 0..2 {
            if client.is_none() {
                client = if attempt == 0 { connect_or_launch().await } else { try_connect().await };
            }
            let Some(c) = client.as_mut() else { break };
            match c.request(&frame).await {
                Ok(r) => {
                    response = Some(r);
                    break;
                }
                Err(e) => {
                    log(&format!("bridge request failed: {e}"));
                    client = None;
                }
            }
        }
        let out = match response {
            Some(r) => framing::write_frame(&mut stdout, &r),
            None => reply(&mut stdout, &unavailable(env.id.as_deref())),
        };
        if out.is_err() {
            return 1;
        }
    }
}

fn host_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nexa-native-host"))
}

fn collect(args: &[String], flag: &str) -> Vec<String> {
    args.windows(2).filter(|w| w[0] == flag).map(|w| w[1].clone()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let code = match cmd {
        "--version" => {
            println!("nexa-native-host {} (protocol {})", nexa_core::APP_VERSION, protocol::PROTOCOL_VERSION);
            0
        }
        "--register" => {
            let res = browser::register(&host_path(), &collect(&args, "--extension-id"), &collect(&args, "--firefox-id"));
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            log("registered native messaging host");
            if res.iter().any(|r| r.registered) { 0 } else { 1 }
        }
        "--unregister" => {
            let res = browser::unregister();
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            log("unregistered native messaging host");
            0
        }
        "--status" => {
            println!("{}", serde_json::to_string_pretty(&browser::status(&host_path())).unwrap());
            0
        }
        // Chrome passes the caller origin; Firefox passes the manifest path and add-on id.
        _ => {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
            rt.block_on(run_host())
        }
    };
    std::process::exit(code);
}
