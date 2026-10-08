//! Untrusted-input handling: URL validation, filename sanitisation, path
//! traversal prevention and approved-directory enforcement.

use std::path::{Component, Path, PathBuf};

use crate::error::{DownloadError, Result};

/// Maximum accepted URL length (browsers cap around 2 MB; we are stricter).
pub const MAX_URL_LEN: usize = 16 * 1024;
const MAX_FILENAME_BYTES: usize = 200;

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Schemes the application accepts as download sources.
pub fn validate_url(input: &str) -> Result<url::Url> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DownloadError::invalid_url("URL is empty"));
    }
    if input.len() > MAX_URL_LEN {
        return Err(DownloadError::invalid_url("URL is too long"));
    }
    if input.chars().any(|c| c.is_control()) {
        return Err(DownloadError::invalid_url("URL contains control characters"));
    }
    let url = url::Url::parse(input).map_err(|e| DownloadError::invalid_url("Invalid URL").with_detail(e.to_string()))?;
    match url.scheme() {
        "http" | "https" | "ftp" => {
            if url.host_str().is_none_or(|h| h.is_empty()) {
                return Err(DownloadError::invalid_url("URL has no host"));
            }
            Ok(url)
        }
        "magnet" => {
            let has_xt = url.query_pairs().any(|(k, v)| k == "xt" && v.starts_with("urn:btih:") || k == "xt" && v.starts_with("urn:btmh:"));
            if !has_xt {
                return Err(DownloadError::invalid_url("Magnet link has no info hash"));
            }
            Ok(url)
        }
        other => Err(DownloadError::invalid_url(format!("Unsupported URL scheme: {other}"))),
    }
}

/// Turns an untrusted name into a safe single path component.
///
/// - strips directory components (`../`, `C:\`, `/etc/`)
/// - replaces characters invalid on Windows
/// - avoids reserved device names, trailing dots/spaces, hidden-dot prefix
/// - limits length while keeping the extension
pub fn sanitize_filename(name: &str) -> String {
    // Keep only the last path component, whichever separator was used.
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = last
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            // Unicode bidi overrides can disguise extensions (e.g. "exe.txt").
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).trim_start_matches('.').trim().to_string();
    if s.is_empty() || s == "_" {
        s = "download".to_string();
    }
    let stem_upper = s.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem_upper.as_str()) {
        s = format!("_{s}");
    }
    truncate_keep_extension(&s, MAX_FILENAME_BYTES)
}

fn truncate_keep_extension(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let (stem, ext) = split_ext(s);
    let ext = if ext.len() > 16 { "" } else { ext };
    let budget = max.saturating_sub(ext.len() + 1);
    let mut cut = String::new();
    for c in stem.chars() {
        if cut.len() + c.len_utf8() > budget {
            break;
        }
        cut.push(c);
    }
    if ext.is_empty() {
        cut
    } else {
        format!("{cut}.{ext}")
    }
}

/// Splits `name.ext` into (`name`, `ext`). Handles `.tar.gz` style double extensions.
pub fn split_ext(name: &str) -> (&str, &str) {
    for double in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if name.len() > double.len() && name.to_ascii_lowercase().ends_with(double) {
            let i = name.len() - double.len();
            return (&name[..i], &name[i + 1..]);
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

pub fn extension_of(name: &str) -> String {
    split_ext(name).1.to_ascii_lowercase()
}

/// Derives a filename from the URL path (percent-decoded).
pub fn filename_from_url(url: &url::Url) -> Option<String> {
    let seg = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let decoded = percent_encoding::percent_decode_str(seg).decode_utf8_lossy().to_string();
    let clean = sanitize_filename(&decoded);
    (clean != "download" || decoded == "download").then_some(clean)
}

/// Parses a `Content-Disposition` header (RFC 6266 / RFC 5987).
/// `filename*` takes precedence over `filename`.
pub fn filename_from_content_disposition(header: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;
    for part in split_params(header) {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "filename*" {
            // charset'lang'pct-encoded
            let mut it = value.splitn(3, '\'');
            let charset = it.next().unwrap_or("").to_ascii_lowercase();
            let _lang = it.next();
            if let Some(enc) = it.next() {
                let bytes: Vec<u8> = percent_encoding::percent_decode_str(enc.trim_matches('"')).collect();
                let s = if charset == "utf-8" || charset.is_empty() {
                    String::from_utf8_lossy(&bytes).to_string()
                } else {
                    bytes.iter().map(|&b| b as char).collect()
                };
                extended = Some(s);
            }
        } else if key == "filename" {
            let v = if value.starts_with('"') {
                unquote(value)
            } else {
                value.to_string()
            };
            // Some servers percent-encode the plain filename.
            let v = if v.contains('%') {
                percent_encoding::percent_decode_str(&v).decode_utf8_lossy().to_string()
            } else {
                v
            };
            plain = Some(v);
        }
    }
    extended.or(plain).map(|n| sanitize_filename(&n)).filter(|n| n != "download")
}

fn split_params(header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in header.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => {
                cur.push(c);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unquote(v: &str) -> String {
    let inner = v.trim().trim_start_matches('"');
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    let mut out = String::new();
    let mut esc = false;
    for c in inner.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// What to do when the target file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DuplicatePolicy {
    /// `file.zip` → `file (1).zip`
    Rename,
    Overwrite,
}

/// Returns a filename inside `dir` that does not collide with an existing file
/// (or with an in-progress `.part`) according to `policy`.
pub fn resolve_duplicate(dir: &Path, filename: &str, policy: DuplicatePolicy, reserved: &dyn Fn(&str) -> bool) -> String {
    let taken = |name: &str| {
        reserved(name) || dir.join(name).exists() || dir.join(format!("{name}.part")).exists()
    };
    if policy == DuplicatePolicy::Overwrite || !taken(filename) {
        return filename.to_string();
    }
    let (stem, ext) = split_ext(filename);
    for i in 1..10_000 {
        let candidate = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{stem}-{}.{ext}", uuid::Uuid::new_v4().simple())
}

/// Normalises a path lexically (resolves `.` and `..`) without touching the disk.
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn canonical_or_normalized(p: &Path) -> PathBuf {
    // Canonicalize the deepest existing ancestor so symlinks are resolved,
    // then append the (normalised) remainder.
    let p = normalize_path(p);
    let mut existing = p.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(n) => {
                rest.push(n.to_os_string());
                existing.pop();
            }
            None => break,
        }
    }
    let mut base = dunce_canonicalize(&existing).unwrap_or(existing);
    for r in rest.into_iter().rev() {
        base.push(r);
    }
    base
}

fn dunce_canonicalize(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    #[cfg(windows)]
    {
        // Strip the \\?\ verbatim prefix for comparisons with user paths.
        let s = c.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            if !stripped.starts_with("UNC") {
                return Ok(PathBuf::from(stripped));
            }
        }
    }
    Ok(c)
}

/// Ensures `dir` is one of (or inside one of) the approved download directories.
/// Returns the normalised directory.
pub fn ensure_approved_dir(dir: &Path, approved: &[PathBuf]) -> Result<PathBuf> {
    if !dir.is_absolute() {
        return Err(DownloadError::new(crate::types::ErrorKind::PermissionDenied, "Destination must be an absolute path"));
    }
    let target = canonical_or_normalized(dir);
    for root in approved {
        let root = canonical_or_normalized(root);
        if path_starts_with(&target, &root) {
            return Ok(target);
        }
    }
    Err(DownloadError::new(
        crate::types::ErrorKind::PermissionDenied,
        "Destination folder is not an approved download folder",
    )
    .with_detail(target.display().to_string()))
}

fn path_starts_with(p: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let a = p.to_string_lossy().to_lowercase();
        let b = root.to_string_lossy().to_lowercase();
        Path::new(&a).starts_with(Path::new(&b))
    }
    #[cfg(not(windows))]
    {
        p.starts_with(root)
    }
}

/// Joins a sanitised filename to a directory and verifies the result stays inside it.
pub fn safe_join(dir: &Path, filename: &str) -> Result<PathBuf> {
    let clean = sanitize_filename(filename);
    let joined = dir.join(&clean);
    if joined.parent() != Some(dir) {
        return Err(DownloadError::new(crate::types::ErrorKind::PermissionDenied, "Unsafe file name"));
    }
    Ok(joined)
}

/// Categorisation by file extension; the user can always override.
pub fn category_for(filename: &str, engine: crate::types::EngineKind) -> &'static str {
    use crate::types::EngineKind;
    if engine == EngineKind::Torrent {
        return "Torrents";
    }
    let ext = extension_of(filename);
    let cat = match ext.as_str() {
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts" => "Videos",
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "opus" | "m4a" | "wma" | "alac" => "Music",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "txt" | "rtf" | "epub" | "csv" | "md" => "Documents",
        "exe" | "msi" | "msix" | "appx" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "apk" | "bat" | "iso" | "img" => "Programs",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tar.gz" | "tar.bz2" | "tar.xz" | "tar.zst" | "cab" => "Archives",
        "torrent" => "Torrents",
        _ => "",
    };
    if !cat.is_empty() {
        return cat;
    }
    if engine == EngineKind::Video {
        "Videos"
    } else {
        "General"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_traversal_and_reserved() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..\\..\\Windows\\win.ini"), "win.ini");
        assert_eq!(sanitize_filename("C:\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("a<b>c:d\"e|f?g*h.zip"), "a_b_c_d_e_f_g_h.zip");
        assert_eq!(sanitize_filename("trailing. . "), "trailing");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename("evil\u{202E}gpj.exe"), "evil_gpj.exe");
        assert_eq!(sanitize_filename("tab\tname.txt"), "tab_name.txt");
    }

    #[test]
    fn truncates_long_names_keeping_extension() {
        let long = format!("{}.zip", "a".repeat(500));
        let s = sanitize_filename(&long);
        assert!(s.len() <= MAX_FILENAME_BYTES);
        assert!(s.ends_with(".zip"));
        let unicode = format!("{}.mp4", "ف".repeat(300));
        let s = sanitize_filename(&unicode);
        assert!(s.len() <= MAX_FILENAME_BYTES && s.ends_with(".mp4"));
    }

    #[test]
    fn content_disposition_parsing() {
        assert_eq!(filename_from_content_disposition("attachment; filename=\"report.pdf\"").unwrap(), "report.pdf");
        assert_eq!(filename_from_content_disposition("attachment; filename=plain.txt").unwrap(), "plain.txt");
        assert_eq!(
            filename_from_content_disposition("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%D8%B3%D9%84%D8%A7%D9%85.txt").unwrap(),
            "سلام.txt"
        );
        assert_eq!(filename_from_content_disposition("attachment; filename=\"../../evil.sh\"").unwrap(), "evil.sh");
        assert_eq!(filename_from_content_disposition("attachment; filename=\"semi;colon.zip\"").unwrap(), "semi;colon.zip");
        assert!(filename_from_content_disposition("inline").is_none());
    }

    #[test]
    fn url_filename() {
        let u = url::Url::parse("https://example.com/files/my%20file.zip?x=1").unwrap();
        assert_eq!(filename_from_url(&u).unwrap(), "my file.zip");
        let u = url::Url::parse("https://example.com/").unwrap();
        assert!(filename_from_url(&u).is_none());
    }

    #[test]
    fn url_validation() {
        assert!(validate_url("https://example.com/a.zip").is_ok());
        assert!(validate_url("ftp://example.com/a.zip").is_ok());
        assert!(validate_url("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_url("magnet:?dn=nohash").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("not a url").is_err());
        assert!(validate_url("https://exa\nmple.com").is_err());
        assert!(validate_url("").is_err());
    }

    #[test]
    fn duplicate_rename() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.zip"), b"x").unwrap();
        std::fs::write(dir.path().join("file (1).zip"), b"x").unwrap();
        let none = |_: &str| false;
        assert_eq!(resolve_duplicate(dir.path(), "file.zip", DuplicatePolicy::Rename, &none), "file (2).zip");
        assert_eq!(resolve_duplicate(dir.path(), "file.zip", DuplicatePolicy::Overwrite, &none), "file.zip");
        assert_eq!(resolve_duplicate(dir.path(), "new.zip", DuplicatePolicy::Rename, &none), "new.zip");
        let reserved = |n: &str| n == "new.zip";
        assert_eq!(resolve_duplicate(dir.path(), "new.zip", DuplicatePolicy::Rename, &reserved), "new (1).zip");
        std::fs::write(dir.path().join("arch.tar.gz"), b"x").unwrap();
        assert_eq!(resolve_duplicate(dir.path(), "arch.tar.gz", DuplicatePolicy::Rename, &none), "arch (1).tar.gz");
    }

    #[test]
    fn approved_dirs() {
        let root = tempfile::tempdir().unwrap();
        let approved = vec![root.path().to_path_buf()];
        assert!(ensure_approved_dir(root.path(), &approved).is_ok());
        assert!(ensure_approved_dir(&root.path().join("sub/dir"), &approved).is_ok());
        assert!(ensure_approved_dir(&root.path().join("../escape"), &approved).is_err());
        assert!(ensure_approved_dir(Path::new("relative"), &approved).is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(ensure_approved_dir(other.path(), &approved).is_err());
    }

    #[test]
    fn categories() {
        use crate::types::EngineKind;
        assert_eq!(category_for("a.MP4", EngineKind::Http), "Videos");
        assert_eq!(category_for("a.tar.gz", EngineKind::Http), "Archives");
        assert_eq!(category_for("setup.exe", EngineKind::Http), "Programs");
        assert_eq!(category_for("x", EngineKind::Torrent), "Torrents");
        assert_eq!(category_for("x.bin", EngineKind::Http), "General");
        assert_eq!(category_for("x", EngineKind::Video), "Videos");
    }
}
