//! Versioned browser-integration protocol (see docs/BROWSER_PROTOCOL.md).
//!
//! Request:  `{"version":1,"type":"add_download","id":"opt-correlation","payload":{...}}`
//! Response: `{"version":1,"id":"…","success":true,...}` or
//!           `{"version":1,"id":"…","success":false,"error":{"code":"…","message":"…"}}`
//!
//! Every message is validated here before it reaches the manager. The browser
//! can never pick a destination folder or touch the filesystem.

use serde::Serialize;
use serde_json::{json, Value};

use crate::manager::{AddSource, DownloadManager};
use crate::security;
use crate::types::{AddDownloadRequest, Download, DownloadStatus};

pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum size of one message in either direction.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH: usize = 200;
const MAX_FILENAME_CHARS: usize = 255;
const MAX_REFERRER_CHARS: usize = 4096;
const MAX_ID_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidJson,
    MessageTooLarge,
    UnsupportedVersion,
    UnknownType,
    InvalidPayload,
    InvalidUrl,
    NotFound,
    AppUnavailable,
    Unauthorized,
    Internal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddItem {
    pub url: String,
    pub filename: Option<String>,
    pub referrer: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Ping,
    AddDownload(AddItem),
    AddMultiple(Vec<AddItem>),
    GetStatus { download_id: Option<String> },
    Pause { download_id: String },
    Resume { download_id: String },
    Cancel { download_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
}

impl ProtocolError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub id: Option<String>,
    pub request: Request,
}

fn str_field(p: &Value, key: &str, max: usize, required: bool) -> Result<Option<String>, ProtocolError> {
    match p.get(key) {
        None | Some(Value::Null) => {
            if required {
                Err(ProtocolError::new(ErrorCode::InvalidPayload, format!("Missing field: {key}")))
            } else {
                Ok(None)
            }
        }
        Some(Value::String(s)) => {
            if s.chars().count() > max {
                return Err(ProtocolError::new(ErrorCode::InvalidPayload, format!("Field too long: {key}")));
            }
            Ok(Some(s.clone()))
        }
        Some(_) => Err(ProtocolError::new(ErrorCode::InvalidPayload, format!("Field must be a string: {key}"))),
    }
}

fn parse_item(p: &Value) -> Result<AddItem, ProtocolError> {
    if !p.is_object() {
        return Err(ProtocolError::new(ErrorCode::InvalidPayload, "Item must be an object"));
    }
    let url = str_field(p, "url", security::MAX_URL_LEN, true)?.unwrap();
    security::validate_url(&url).map_err(|e| ProtocolError::new(ErrorCode::InvalidUrl, e.message))?;
    let filename = str_field(p, "filename", MAX_FILENAME_CHARS, false)?
        .map(|f| security::sanitize_filename(&f))
        .filter(|f| f != "download");
    let referrer = str_field(p, "referrer", MAX_REFERRER_CHARS, false)?
        .filter(|r| url::Url::parse(r).map(|u| u.scheme() == "http" || u.scheme() == "https").unwrap_or(false));
    Ok(AddItem { url: url.trim().to_string(), filename, referrer })
}

fn download_id(p: &Value) -> Result<String, ProtocolError> {
    let id = str_field(p, "download_id", MAX_ID_CHARS, true)?.unwrap();
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(ProtocolError::new(ErrorCode::InvalidPayload, "Invalid download_id"));
    }
    Ok(id)
}

/// Parses and validates raw message bytes.
pub fn parse(raw: &[u8]) -> Result<Envelope, (Option<String>, ProtocolError)> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err((None, ProtocolError::new(ErrorCode::MessageTooLarge, "Message too large")));
    }
    let v: Value = serde_json::from_slice(raw).map_err(|_| (None, ProtocolError::new(ErrorCode::InvalidJson, "Malformed JSON")))?;
    let obj = v.as_object().ok_or((None, ProtocolError::new(ErrorCode::InvalidPayload, "Message must be a JSON object")))?;
    let id = match obj.get("id") {
        Some(Value::String(s)) if s.len() <= MAX_ID_CHARS => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let err = |e: ProtocolError| (id.clone(), e);
    let version = obj.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != PROTOCOL_VERSION as u64 {
        return Err(err(ProtocolError::new(ErrorCode::UnsupportedVersion, format!("Unsupported protocol version {version}; this app speaks version {PROTOCOL_VERSION}"))));
    }
    let ty = obj.get("type").and_then(|t| t.as_str()).ok_or_else(|| err(ProtocolError::new(ErrorCode::InvalidPayload, "Missing type")))?;
    let empty = json!({});
    let payload = obj.get("payload").unwrap_or(&empty);
    if !payload.is_object() {
        return Err(err(ProtocolError::new(ErrorCode::InvalidPayload, "payload must be an object")));
    }
    let request = match ty {
        "ping" => Request::Ping,
        "add_download" => Request::AddDownload(parse_item(payload).map_err(err)?),
        "add_multiple_downloads" => {
            let items = payload.get("items").and_then(|i| i.as_array()).ok_or_else(|| err(ProtocolError::new(ErrorCode::InvalidPayload, "items must be an array")))?;
            if items.is_empty() || items.len() > MAX_BATCH {
                return Err(err(ProtocolError::new(ErrorCode::InvalidPayload, format!("items must contain 1–{MAX_BATCH} entries"))));
            }
            Request::AddMultiple(items.iter().map(parse_item).collect::<Result<_, _>>().map_err(err)?)
        }
        "get_status" => Request::GetStatus {
            download_id: match payload.get("download_id") {
                None | Some(Value::Null) => None,
                _ => Some(download_id(payload).map_err(err)?),
            },
        },
        "pause" => Request::Pause { download_id: download_id(payload).map_err(err)? },
        "resume" => Request::Resume { download_id: download_id(payload).map_err(err)? },
        "cancel" => Request::Cancel { download_id: download_id(payload).map_err(err)? },
        other => return Err(err(ProtocolError::new(ErrorCode::UnknownType, format!("Unknown message type: {}", other.chars().take(40).collect::<String>())))),
    };
    Ok(Envelope { id, request })
}

pub fn error_response(id: Option<&str>, e: &ProtocolError) -> Value {
    json!({
        "version": PROTOCOL_VERSION,
        "id": id,
        "success": false,
        "error": { "code": e.code, "message": e.message },
    })
}

fn ok(id: Option<&str>, mut extra: Value) -> Value {
    let o = extra.as_object_mut().expect("object");
    o.insert("version".into(), json!(PROTOCOL_VERSION));
    o.insert("id".into(), json!(id));
    o.insert("success".into(), json!(true));
    extra
}

/// Public view of a download for the extension (no local paths).
fn status_view(d: &Download) -> Value {
    json!({
        "download_id": d.id,
        "filename": d.filename,
        "status": d.status,
        "downloaded_bytes": d.downloaded_bytes,
        "total_bytes": d.total_bytes,
        "speed_bytes_per_second": d.speed_bps,
        "eta_seconds": d.eta_seconds,
        "error": d.error_message,
    })
}

/// Executes a raw message against the manager and returns the JSON response.
pub async fn handle(mgr: &DownloadManager, raw: &[u8]) -> Value {
    let env = match parse(raw) {
        Ok(e) => e,
        Err((id, e)) => return error_response(id.as_deref(), &e),
    };
    let id = env.id.as_deref();
    if !mgr.settings().browser_integration && env.request != Request::Ping {
        return error_response(id, &ProtocolError::new(ErrorCode::Unauthorized, "Browser integration is disabled in pixidl settings"));
    }
    let add = |item: AddItem| AddDownloadRequest { url: item.url, filename: item.filename, referrer: item.referrer, ..Default::default() };
    let not_found = || ProtocolError::new(ErrorCode::NotFound, "Download not found");
    match env.request {
        Request::Ping => ok(id, json!({ "app": crate::APP_NAME, "app_version": crate::APP_VERSION, "protocol_version": PROTOCOL_VERSION })),
        Request::AddDownload(item) => match mgr.add(add(item), AddSource::Browser).await {
            Ok(d) => ok(id, json!({ "download_id": d.id, "filename": d.filename, "engine": d.engine })),
            Err(e) => error_response(id, &ProtocolError::new(if e.kind == crate::types::ErrorKind::InvalidUrl { ErrorCode::InvalidUrl } else { ErrorCode::Internal }, e.message)),
        },
        Request::AddMultiple(items) => {
            let mut results = Vec::with_capacity(items.len());
            for item in items {
                let url = item.url.clone();
                results.push(match mgr.add(add(item), AddSource::Browser).await {
                    Ok(d) => json!({ "url": url, "success": true, "download_id": d.id }),
                    Err(e) => json!({ "url": url, "success": false, "error": e.message }),
                });
            }
            let added = results.iter().filter(|r| r["success"] == json!(true)).count();
            ok(id, json!({ "added": added, "results": results }))
        }
        Request::GetStatus { download_id } => match download_id {
            Some(did) => match mgr.get(&did) {
                Ok(Some(d)) => ok(id, json!({ "download": status_view(&d) })),
                _ => error_response(id, &not_found()),
            },
            None => match (mgr.stats(), mgr.list()) {
                (Ok(s), Ok(list)) => {
                    let active: Vec<Value> = list.iter().filter(|d| d.status.is_running() || d.status == DownloadStatus::Queued).take(50).map(status_view).collect();
                    ok(id, json!({ "active": s.active, "queued": s.queued, "download_bytes_per_second": s.download_bps, "downloads": active }))
                }
                _ => error_response(id, &ProtocolError::new(ErrorCode::Internal, "Status unavailable")),
            },
        },
        Request::Pause { download_id } | Request::Resume { download_id } | Request::Cancel { download_id } if mgr.get(&download_id).ok().flatten().is_none() => {
            error_response(id, &not_found())
        }
        Request::Pause { download_id } => match mgr.pause(&download_id) {
            Ok(()) => ok(id, json!({ "download_id": download_id })),
            Err(e) => error_response(id, &ProtocolError::new(ErrorCode::Internal, e.message)),
        },
        Request::Resume { download_id } => match mgr.resume(&download_id) {
            Ok(()) => ok(id, json!({ "download_id": download_id })),
            Err(e) => error_response(id, &ProtocolError::new(ErrorCode::Internal, e.message)),
        },
        Request::Cancel { download_id } => match mgr.cancel(&download_id).await {
            Ok(()) => ok(id, json!({ "download_id": download_id })),
            Err(e) => error_response(id, &ProtocolError::new(ErrorCode::Internal, e.message)),
        },
    }
}

/// Native-messaging framing: 4-byte length (native byte order) + UTF-8 JSON.
pub mod framing {
    use super::MAX_MESSAGE_BYTES;
    use std::io::{self, Read, Write};

    pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
        let mut len = [0u8; 4];
        match r.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let n = u32::from_ne_bytes(len) as usize;
        if n > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }
        let mut buf = vec![0u8; n];
        r.read_exact(&mut buf)?;
        Ok(Some(buf))
    }

    pub fn write_frame(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
        if data.len() > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }
        w.write_all(&(data.len() as u32).to_ne_bytes())?;
        w.write_all(data)?;
        w.flush()
    }

    pub async fn read_frame_async<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
        use tokio::io::AsyncReadExt;
        let mut len = [0u8; 4];
        match r.read_exact(&mut len).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let n = u32::from_le_bytes(len) as usize;
        if n > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }
        let mut buf = vec![0u8; n];
        r.read_exact(&mut buf).await?;
        Ok(Some(buf))
    }

    pub async fn write_frame_async<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        if data.len() > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }
        w.write_all(&(data.len() as u32).to_le_bytes()).await?;
        w.write_all(data).await?;
        w.flush().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: Value) -> Result<Envelope, (Option<String>, ProtocolError)> {
        parse(v.to_string().as_bytes())
    }

    #[test]
    fn parses_valid_messages() {
        let e = p(json!({"version":1,"type":"ping","id":"a1"})).unwrap();
        assert_eq!(e.request, Request::Ping);
        assert_eq!(e.id.as_deref(), Some("a1"));
        let e = p(json!({"version":1,"type":"add_download","payload":{"url":"https://example.com/f.zip","filename":"../../x.exe","referrer":"https://example.com/page"}})).unwrap();
        assert_eq!(
            e.request,
            Request::AddDownload(AddItem { url: "https://example.com/f.zip".into(), filename: Some("x.exe".into()), referrer: Some("https://example.com/page".into()) })
        );
        let e = p(json!({"version":1,"type":"add_multiple_downloads","payload":{"items":[{"url":"https://a.com/1"},{"url":"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"}]}})).unwrap();
        assert!(matches!(e.request, Request::AddMultiple(ref v) if v.len() == 2));
        let e = p(json!({"version":1,"type":"pause","payload":{"download_id":"0b3c-11"}})).unwrap();
        assert_eq!(e.request, Request::Pause { download_id: "0b3c-11".into() });
        let e = p(json!({"version":1,"type":"get_status"})).unwrap();
        assert_eq!(e.request, Request::GetStatus { download_id: None });
    }

    #[test]
    fn rejects_invalid_messages() {
        let code = |r: Result<Envelope, (Option<String>, ProtocolError)>| r.unwrap_err().1.code;
        assert_eq!(code(parse(b"{not json")), ErrorCode::InvalidJson);
        assert_eq!(code(parse(b"[1,2]")), ErrorCode::InvalidPayload);
        assert_eq!(code(p(json!({"version":2,"type":"ping"}))), ErrorCode::UnsupportedVersion);
        assert_eq!(code(p(json!({"type":"ping"}))), ErrorCode::UnsupportedVersion);
        assert_eq!(code(p(json!({"version":1,"type":"rm_rf"}))), ErrorCode::UnknownType);
        assert_eq!(code(p(json!({"version":1,"type":"add_download","payload":{}}))), ErrorCode::InvalidPayload);
        assert_eq!(code(p(json!({"version":1,"type":"add_download","payload":{"url":"file:///etc/passwd"}}))), ErrorCode::InvalidUrl);
        assert_eq!(code(p(json!({"version":1,"type":"add_download","payload":{"url":5}}))), ErrorCode::InvalidPayload);
        assert_eq!(code(p(json!({"version":1,"type":"add_download","payload":"x"}))), ErrorCode::InvalidPayload);
        assert_eq!(code(p(json!({"version":1,"type":"add_multiple_downloads","payload":{"items":[]}}))), ErrorCode::InvalidPayload);
        let many: Vec<Value> = (0..201).map(|i| json!({"url": format!("https://a.com/{i}")})).collect();
        assert_eq!(code(p(json!({"version":1,"type":"add_multiple_downloads","payload":{"items":many}}))), ErrorCode::InvalidPayload);
        assert_eq!(code(p(json!({"version":1,"type":"pause","payload":{"download_id":"../../etc"}}))), ErrorCode::InvalidPayload);
        let huge = vec![b' '; MAX_MESSAGE_BYTES + 1];
        assert_eq!(code(parse(&huge)), ErrorCode::MessageTooLarge);
        // The correlation id survives errors.
        let (id, _) = p(json!({"version":1,"type":"nope","id":"x9"})).unwrap_err();
        assert_eq!(id.as_deref(), Some("x9"));
    }

    #[test]
    fn non_http_referrer_dropped() {
        let e = p(json!({"version":1,"type":"add_download","payload":{"url":"https://a.com/x","referrer":"javascript:alert(1)"}})).unwrap();
        assert!(matches!(e.request, Request::AddDownload(AddItem { referrer: None, .. })));
    }

    #[test]
    fn framing_round_trip_and_limits() {
        let mut buf = Vec::new();
        framing::write_frame(&mut buf, br#"{"a":1}"#).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        assert_eq!(framing::read_frame(&mut cur).unwrap().unwrap(), br#"{"a":1}"#);
        assert!(framing::read_frame(&mut cur).unwrap().is_none());
        let mut bad = ((MAX_MESSAGE_BYTES + 1) as u32).to_ne_bytes().to_vec();
        bad.extend_from_slice(b"xx");
        assert!(framing::read_frame(&mut std::io::Cursor::new(bad)).is_err());
    }
}
