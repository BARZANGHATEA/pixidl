# Troubleshooting

## Where things are

| | Windows | Linux |
|---|---|---|
| Database, settings, bridge file | `%APPDATA%\com.nexa.downloadmanager\` | `~/.local/share/com.nexa.downloadmanager/` |
| Logs (JSON lines, rotated daily, 7 kept) | `…\logs\nexa.YYYY-MM-DD.log` | same, under the data folder |
| Native host log | `…\logs\native-host.log` | same |
| Installed program | `%LOCALAPPDATA%\Nexa Download Manager\` | — |

Settings → Engines → **Open logs folder** (or About → Open logs folder) opens the
logs folder. Logs never contain passwords, cookies, tokens or URL query strings.
Set the environment variable `NEXA_LOG=debug` for more detail.

## Downloads

**"Network unavailable" / "Connection timed out"**
- Check the proxy in Settings → Connection. *System proxy* honours the
  `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY` variables. Manual accepts `http://`,
  `https://` and `socks5://`.
- Behind a corporate TLS-inspecting proxy, its root certificate must be in the
  Windows certificate store. Nexa trusts the OS store as well as the public roots.
- Transient errors are retried automatically (Settings → Connection → Automatic
  retries). The row shows "Retrying (n)".

**"Server rejected the request (access denied)"**
The server answered 401 or 403. Many sites need cookies from a logged-in session,
which Nexa deliberately does not import. Download such files in the browser.

**"Resume not supported" / the download restarted from 0 %**
The server ignores byte ranges, or the file changed on the server (its ETag or
Last-Modified no longer matches). Nexa restarts instead of producing a corrupt
file. The *Details* activity log records why.

**"Disk full"**
Nexa checks free space before starting and stops cleanly when the disk fills. Free
space, then press Retry; the download continues from the partial file.

**"Destination folder is not an approved download folder"**
For safety, files can only be written to the default folder, its subfolders, or
folders you picked with the folder button (Settings → Downloads → Other allowed
folders).

**A completed download says "File missing"**
The file was moved or deleted outside Nexa. Remove the entry, or Retry to download
it again.

## Torrents

- **Magnet stuck in "Preparing… Fetching metadata"**: no peer has provided the
  metadata yet. Make sure DHT is enabled (Settings → Engines) and that your network
  allows outgoing UDP/TCP. A magnet with no trackers and DHT disabled fails
  immediately with "Torrent metadata unavailable".
- **Few or no peers**: set a fixed listening port (Settings → Engines) and allow it
  in the Windows firewall or forward it on your router.
- Seeds are not shown separately because the torrent library does not report them.
  Nexa only shows real numbers.

## Video downloads

- **"Video engine (yt-dlp) is not installed"**: Settings → Engines → *Install
  yt-dlp*. It downloads the official release from GitHub and verifies its SHA-256
  checksum. You can also set a custom path.
- **"Extractor failed" / "This site is not supported"**: sites change often. Use
  *Check for updates* in Settings → Engines; the full yt-dlp message is under
  *Details*.
- **Only low qualities offered / "FFmpeg is required"**: install FFmpeg (for
  example `winget install Gyan.FFmpeg`) or set its path in Settings → Engines. Most
  high-quality streams store video and audio separately and need FFmpeg to merge
  them.
- **Private or members-only videos** need an account. Nexa does not handle
  credentials.

## Browser integration

1. Settings → Browser integration should show **Native messaging host:
   Installed**, **Local bridge: Running** and **Registered** for your browser. If
   not, click **Reinstall integration**.
2. The extension ID must be allowed. The reference extension
   (`ndlafmjbcbcjmkegfelbhgknmajgdbna`) always is. Add your own extension's ID
   under *Additional Chromium extension IDs* (or Firefox add-on IDs).
3. Restart the browser after the first registration; browsers read the host
   manifests at startup.
4. The popup says "Nexa is not running": the host tries to start Nexa in the
   background automatically. Check `native-host.log` if that fails.
5. From a terminal: `"%LOCALAPPDATA%\Nexa Download Manager\nexa-native-host.exe"
   --status` prints the registration of every browser as JSON.

## App does not start

- Missing WebView2: install the Evergreen runtime from Microsoft. The installer
  normally does this.
- **"Could not open the download database"**: another copy may be locked by
  antivirus, or the data folder is not writable. Check the logs. To start fresh,
  close Nexa and rename the data folder; your downloaded files are not affected.
- Only one instance runs at a time. Launching again focuses the running window,
  which may be hidden in the tray.

## Reporting a bug

Include the app version (About), the relevant lines from the newest log file and
the *Details* section of the failed download, with its technical details expanded.
