# Building Nexa Download Manager

## Prerequisites

| | Windows 10/11 (target platform) | Linux (development) |
|---|---|---|
| Node.js | 22 or newer | 22 or newer |
| Rust | stable (`rustup default stable`) | stable |
| C toolchain | Visual Studio 2022 Build Tools, workload **Desktop development with C++** | `build-essential` |
| WebView | WebView2 (preinstalled on Windows 10/11; the installer bootstraps it if missing) | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config` |
| Optional | yt-dlp and FFmpeg on `PATH` for the video tests | same |

Install the JavaScript dependencies once:

```bash
npm install
```

## Development

```bash
npm run dev
```

This builds the native messaging host sidecar in debug mode
(`src-tauri/binaries/nexa-native-host-<target-triple>`), starts Vite on port 1420
and launches the app with hot reload. App data goes to
`%APPDATA%\com.nexa.downloadmanager` (Linux: `~/.local/share/com.nexa.downloadmanager`).
Set `NEXA_DATA_DIR` to use a separate data folder, and `NEXA_LOG=debug` for verbose logs.

Without a desktop app you can work on the UI alone with `npm run dev:web`. Backend
calls fail there because there is no Tauri runtime.

## Tests

```bash
npm run typecheck
npm test                                   # 35 Vitest tests (components, filters, i18n parity, ...)
node --test browser-extension/test/        # extension helpers, manifest, locales
node scripts/build-native-host.mjs --debug # required once before compiling the app crate
cargo test --workspace                     # Rust unit and integration tests
cargo clippy --workspace --all-targets -- -D warnings
```

Notes:

- `cargo test` compiles `src-tauri`, and Tauri checks that the sidecar binary
  exists. That is why the sidecar must be built first.
- The video tests print `SKIPPED` when yt-dlp or FFmpeg is not installed.
- The torrent test runs entirely on localhost: a seeder session, a minimal HTTP
  tracker and the manager.

Regenerate the TypeScript types after changing shared Rust structs:

```bash
npm run gen:types     # ts-rs → src/types/generated/
```

## Production build

```bash
npm run build
```

This builds the release sidecar and runs `tauri build` for the current platform.

## Windows installer

```bash
npm run package:windows
```

1. `scripts/fetch-engines.mjs` downloads the official yt-dlp release
   (`yt-dlp.exe`) and verifies it against the release's `SHA2-256SUMS`. If the
   checksums do not match, the build fails. Pin a version with
   `YTDLP_VERSION=2026.08.19`.
2. `scripts/build-native-host.mjs` builds `nexa-native-host.exe` (release).
3. `tauri build --bundles nsis` builds the NSIS installer at
   `target/release/bundle/nsis/Nexa Download Manager_<version>_x64-setup.exe`.

What the installer does:

- installs per user to `%LOCALAPPDATA%\Nexa Download Manager` (no admin rights
  needed) and creates the Start menu shortcut
- installs `nexa-native-host.exe` and the bundled `bin\yt-dlp.exe`
- runs `nexa-native-host.exe --register` (`installer/hooks.nsh`), which registers
  the host for Chrome, Edge, Brave, Chromium, Vivaldi and Firefox
- associates `.torrent` files with the app
- on uninstall, runs `--unregister`, stops the running app and removes the
  program files. Downloads, settings and history in `%APPDATA%` are kept unless
  the user ticks *Delete the application data*.

**FFmpeg is not bundled**, for licensing and size reasons. The app detects FFmpeg on
`PATH` (for example `winget install Gyan.FFmpeg`) or at a path set in Settings →
Engines. Without FFmpeg, the video engine only offers formats that already contain
both audio and video.

### Code signing

Unsigned installers trigger SmartScreen warnings. To sign:

- set `bundle.windows.certificateThumbprint`, `digestAlgorithm` and
  `timestampUrl` in `src-tauri/tauri.conf.json`, or
- set `bundle.windows.signCommand` for Azure Trusted Signing / a cloud HSM.

### Updates

The version lives in two places: `Cargo.toml` (`workspace.package.version`, which
the app reports) and `src-tauri/tauri.conf.json` / `package.json`. The app shows it
under About.

Automatic updates are deliberately not enabled. To add them, use
`tauri-plugin-updater`, which only installs update artifacts signed with your
updater key (`tauri signer generate`). Publish the signed `latest.json` and installer
on your releases page, and add the endpoint and public key to `tauri.conf.json`.

## Continuous integration

`.github/workflows/ci.yml` has two jobs.

**Linux:** runs typecheck, the frontend and extension tests, clippy with warnings as
errors, and every Rust test, including the real yt-dlp and torrent engines.

**Windows:** runs the test suite, then:

1. builds the installer
2. installs it silently and checks the files and registry keys
3. starts the installed app through the installed native host and downloads a real file
4. uninstalls silently and checks that the browser integration was removed
5. uploads the installer as an artifact

## Third-party notices

```bash
npm run gen:notices
```

Regenerates `src-tauri/resources/THIRD-PARTY-NOTICES.md` from `cargo metadata` and
the npm production dependencies. Run it after changing dependencies.
