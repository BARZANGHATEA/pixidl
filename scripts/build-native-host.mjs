// Builds the native-messaging host and places it where Tauri expects a
// sidecar ("externalBin"): src-tauri/binaries/nexa-native-host-<target-triple>[.exe]
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = !process.argv.includes("--debug");
const rustInfo = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || /host: (\S+)/.exec(rustInfo)[1];
const ext = triple.includes("windows") ? ".exe" : "";

const args = ["build", "-p", "nexa-native-host", "--target", triple];
if (release) args.push("--release");
console.log(`> cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const built = join(root, "target", triple, release ? "release" : "debug", `nexa-native-host${ext}`);
if (!existsSync(built)) throw new Error(`native host not found at ${built}`);
const outDir = join(root, "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, `nexa-native-host-${triple}${ext}`);
copyFileSync(built, out);
console.log(`native host -> ${out}`);
