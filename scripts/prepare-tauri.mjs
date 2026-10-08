// Everything the Tauri app needs before it can compile or be bundled:
//   1. the browser extension packages (browser-extension/dist → bundled as resources)
//   2. the native messaging host sidecar
// Usage: node scripts/prepare-tauri.mjs [--debug]
import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const run = (script, args = []) => execFileSync(process.execPath, [join(root, script), ...args], { cwd: root, stdio: "inherit" });
run("browser-extension/build.mjs");
run("scripts/build-native-host.mjs", process.argv.slice(2));
