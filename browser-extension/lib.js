// Pure helpers shared by the background script and the popup.
// No browser globals are touched at import time, so this module is testable
// in Node (see test/native.test.mjs).

export const HOST_NAME = "com.nexa.downloadmanager";
export const PROTOCOL_VERSION = 1;
/** Maximum items per add_multiple_downloads message (enforced by the app). */
export const MAX_BATCH = 200;
/**
 * Byte budget for one batch request. The host caps messages at 1 MiB in each
 * direction and the response echoes every URL, so stay well below that.
 */
export const MAX_BATCH_BYTES = 512 * 1024;
/** Same limit as the app's URL validator. */
export const MAX_URL_LEN = 16 * 1024;
/** Upper bound on links collected from one page. */
export const MAX_LINKS = 5000;

/** A correlation id for one request (at most 128 chars per the protocol). */
export function makeId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

/** Builds a protocol request envelope. */
export function buildEnvelope(type, payload = {}, id = makeId()) {
  return { version: PROTOCOL_VERSION, type, id, payload };
}

/** A failure response in the same shape the app uses. */
export function errorResponse(code, message, id = null) {
  return { version: PROTOCOL_VERSION, id, success: false, error: { code, message } };
}

/** Guarantees the caller always gets `{success, ...}` with an `error` on failure. */
export function normalizeResponse(resp, id = null) {
  if (!resp || typeof resp !== "object" || Array.isArray(resp)) {
    return errorResponse("internal", "Invalid response from the native host", id);
  }
  if (resp.success === true) return resp;
  const error = resp.error && typeof resp.error === "object" ? resp.error : {};
  return {
    ...resp,
    success: false,
    error: { code: String(error.code || "internal"), message: String(error.message || "Unknown error") },
  };
}

/**
 * Returns the normalized URL string if Nexa can download it, otherwise null.
 * Accepted: http(s) with a host, and magnet links carrying an info hash.
 * ftp is deliberately not sent (the app currently rejects it).
 */
export function supportedUrl(input) {
  if (typeof input !== "string") return null;
  const raw = input.trim();
  if (!raw || raw.length > MAX_URL_LEN) return null;
  let url;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol === "http:" || url.protocol === "https:") {
    if (!url.hostname) return null;
    url.hash = "";
    return url.href;
  }
  if (url.protocol === "magnet:") {
    const hasHash = new URLSearchParams(url.search).getAll("xt").some((xt) => /^urn:bt(ih|mh):/i.test(xt));
    return hasHash ? raw : null;
  }
  return null;
}

/** True for plain http(s) URLs (the only kind captured from browser downloads). */
export function isHttpUrl(input) {
  const url = supportedUrl(input);
  return url !== null && /^https?:/i.test(url);
}

/** A referrer is only forwarded when it is an http(s) page. */
export function safeReferrer(input) {
  return isHttpUrl(input) ? supportedUrl(input) : undefined;
}

/**
 * Validates, normalizes (fragment removed) and de-duplicates a list of URLs,
 * keeping the first-seen order. `exclude` drops e.g. the page's own URL.
 */
export function normalizeLinks(urls, { exclude = [], limit = MAX_LINKS } = {}) {
  const skip = new Set(exclude.map(supportedUrl).filter(Boolean));
  const seen = new Set();
  const out = [];
  for (const candidate of Array.isArray(urls) ? urls : []) {
    const url = supportedUrl(candidate);
    if (!url || skip.has(url) || seen.has(url)) continue;
    seen.add(url);
    out.push(url);
    if (out.length >= limit) break;
  }
  return out;
}

/** "zip, .PDF *.tar.gz" -> ["zip", "pdf", "tar.gz"] */
export function parseExtensionFilter(text) {
  const out = [];
  for (const part of String(text ?? "").split(/[\s,;|]+/)) {
    const ext = part.trim().toLowerCase().replace(/^\*?\.+/, "");
    if (/^[a-z0-9][a-z0-9.+_-]{0,15}$/.test(ext) && !out.includes(ext)) out.push(ext);
  }
  return out;
}

/** Last path segment of a URL, decoded when possible. */
export function urlFileName(input) {
  try {
    const url = new URL(input);
    const segment = url.pathname.split("/").pop() || "";
    try {
      return decodeURIComponent(segment);
    } catch {
      return segment;
    }
  } catch {
    return "";
  }
}

/** True when the URL's file name ends with one of the extensions (or no filter is set). */
export function matchesExtensions(url, extensions) {
  if (!extensions || extensions.length === 0) return true;
  const name = urlFileName(url).toLowerCase();
  return extensions.some((ext) => name.endsWith(`.${ext}`));
}

/** File name part of a local path (either separator); "" when there is none. */
export function basename(path) {
  if (typeof path !== "string") return "";
  return path.split(/[\\/]/).pop().trim();
}

/** A protocol item `{url, filename?, referrer?}` with empty fields omitted. */
export function buildItem(url, { filename, referrer } = {}) {
  const item = { url };
  const name = basename(filename);
  if (name) item.filename = name.slice(0, 255);
  const ref = safeReferrer(referrer);
  if (ref) item.referrer = ref.slice(0, 4096);
  return item;
}

/**
 * Splits items into batches of at most `maxCount` entries whose JSON size
 * stays under `maxBytes` (a single oversized item still gets its own batch).
 */
export function batchItems(items, maxCount = MAX_BATCH, maxBytes = MAX_BATCH_BYTES) {
  const batches = [];
  let current = [];
  let size = 0;
  for (const item of items) {
    const itemSize = JSON.stringify(item).length + 1;
    if (current.length > 0 && (current.length >= maxCount || size + itemSize > maxBytes)) {
      batches.push(current);
      current = [];
      size = 0;
    }
    current.push(item);
    size += itemSize;
  }
  if (current.length > 0) batches.push(current);
  return batches;
}

/** Integer percentage, or null when the total size is unknown. */
export function percent(downloaded, total) {
  if (!Number.isFinite(total) || total <= 0 || !Number.isFinite(downloaded)) return null;
  return Math.max(0, Math.min(100, Math.floor((downloaded / total) * 100)));
}

/** Human-readable byte size, e.g. 1536 -> "1.5 KB". */
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

/** Whether a captured browser download is large enough to hand to Nexa. */
export function meetsMinSize(sizeBytes, minSizeMb) {
  const min = Number(minSizeMb);
  if (!Number.isFinite(min) || min <= 0) return true;
  if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) return true; // unknown size: capture
  return sizeBytes >= min * 1024 * 1024;
}

/** Stored options (chrome.storage.local). Capturing is off until the user opts in. */
export const DEFAULT_SETTINGS = Object.freeze({ captureDownloads: false, minSizeMb: 0 });

/** Coerces stored options into valid values. */
export function sanitizeSettings(stored) {
  const min = Math.floor(Number(stored?.minSizeMb));
  return {
    captureDownloads: stored?.captureDownloads === true,
    minSizeMb: Number.isFinite(min) ? Math.min(Math.max(min, 0), 1_000_000) : 0,
  };
}
