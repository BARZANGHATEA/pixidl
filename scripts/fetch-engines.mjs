// Fetches the official yt-dlp release binary into src-tauri/resources/bin so it
// is bundled with the installer. yt-dlp is released under the Unlicense.
//
// The binary is verified against the SHA2-256SUMS file published with the same
// release before it is written. Pin a version with YTDLP_VERSION=2026.08.19,
// otherwise the latest release is used. FFmpeg is NOT bundled (licensing and
// size); the app detects a system FFmpeg or a user-configured path.
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync, chmodSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const target = process.env.TAURI_ENV_TARGET_TRIPLE || process.argv[2] || `${process.arch}-${process.platform}`;
const isWin = /windows|win32/.test(target);
const isMac = /apple|darwin/.test(target);
const isArm = /aarch64|arm64/.test(target);
const asset = isWin ? "yt-dlp.exe" : isMac ? "yt-dlp_macos" : isArm ? "yt-dlp_linux_aarch64" : "yt-dlp_linux";
const version = process.env.YTDLP_VERSION;
const base = version
  ? `https://github.com/yt-dlp/yt-dlp/releases/download/${version}`
  : "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

async function get(url) {
  const r = await fetch(url, { redirect: "follow" });
  if (!r.ok) throw new Error(`GET ${url} → HTTP ${r.status}`);
  return Buffer.from(await r.arrayBuffer());
}

const sums = (await get(`${base}/SHA2-256SUMS`)).toString("utf8");
const line = sums.split("\n").find((l) => l.trim().split(/\s+/)[1]?.replace(/^\*/, "") === asset);
if (!line) throw new Error(`no checksum for ${asset} in SHA2-256SUMS`);
const expected = line.trim().split(/\s+/)[0].toLowerCase();
const bin = await get(`${base}/${asset}`);
const actual = createHash("sha256").update(bin).digest("hex");
if (actual !== expected) throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);

const outDir = join(root, "src-tauri", "resources", "bin");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, isWin ? "yt-dlp.exe" : "yt-dlp");
writeFileSync(out, bin);
if (!isWin) chmodSync(out, 0o755);
console.log(`yt-dlp (${version ?? "latest"}, ${asset}) verified sha256=${actual} -> ${out}`);
