// Formatting helpers. All values come from the backend; nothing is estimated here
// except the percentage, which is computed from real byte counts.

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number | null | undefined, locale = "en"): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes) || bytes < 0) return "—";
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i === 0 ? 0 : v >= 100 ? 0 : 1;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(v)} ${UNITS[i]}`;
}

export function formatSpeed(bps: number | null | undefined, locale = "en"): string {
  if (!bps || bps <= 0) return `0 B/s`;
  return `${formatBytes(bps, locale)}/s`;
}

/** 134 → "2m 14s", 3700 → "1h 1m", 45 → "45s". */
export function formatEta(seconds: number | null | undefined): string | null {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) return null;
  const s = Math.round(seconds);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec.toString().padStart(2, "0")}s`;
  return `${sec}s`;
}

export function percent(downloaded: number, total: number | null | undefined): number | null {
  if (!total || total <= 0) return null;
  return Math.max(0, Math.min(100, (downloaded / total) * 100));
}

export function formatPercent(p: number | null): string {
  if (p === null) return "";
  return `${Math.floor(p)}%`;
}

export function formatDuration(seconds: number | null | undefined): string | null {
  if (!seconds || seconds <= 0) return null;
  const s = Math.round(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}` : `${m}:${sec.toString().padStart(2, "0")}`;
}

/** "12s ago", "5m ago", "3h ago", otherwise a short date. */
export function formatRelative(iso: string | null | undefined, now = Date.now(), locale = "en"): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const diff = Math.max(0, Math.round((now - t) / 1000));
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto", style: "short" });
  if (diff < 60) return rtf.format(-diff, "second");
  if (diff < 3600) return rtf.format(-Math.floor(diff / 60), "minute");
  if (diff < 86400) return rtf.format(-Math.floor(diff / 3600), "hour");
  return formatDate(iso, locale);
}

export function formatDate(iso: string | null | undefined, locale = "en"): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(d);
}

/** Parses "10", "1.5" (MB/s) into bytes/s; empty → null (unlimited). */
export function parseLimitMBps(input: string): number | null {
  const v = Number.parseFloat(input.replace(",", "."));
  if (!Number.isFinite(v) || v <= 0) return null;
  return Math.round(v * 1024 * 1024);
}

export function limitToMBps(bps: number | null | undefined): string {
  if (!bps) return "";
  return String(Math.round((bps / 1024 / 1024) * 100) / 100);
}

export function hostOf(url: string): string {
  try {
    const u = new URL(url);
    if (u.protocol === "magnet:") return "magnet";
    return u.host;
  } catch {
    return "";
  }
}
