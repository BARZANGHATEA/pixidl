# Nexa Download Manager browser extension

The reference extension for Nexa Download Manager. It sends links, media and
(optionally) browser downloads to the Nexa desktop app through
[Native Messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging).
It is a Manifest V3 extension for Chromium browsers (Chrome, Edge, Brave,
Vivaldi) and Firefox 128 or newer, written in plain ES modules with no build step.

## Features

- **Context menus**: "Download with Nexa" on links, "Download media with Nexa"
  on images, video and audio, "Send this page to Nexa" on pages (Nexa's
  extractor handles media pages), and "Download all links in selection with Nexa".
- **Popup**: connection status, "Send current page", "Download all links on
  this page" with an optional file-type filter (for example `zip, pdf`), and the
  active downloads with pause, resume and cancel.
- **Capture browser downloads** (off by default, under Options in the popup):
  http(s) downloads started in the browser are handed to Nexa. The browser
  download is cancelled only after Nexa has accepted it. If Nexa is not running
  or rejects the link, the browser keeps downloading as usual. `blob:` and
  `data:` downloads and private-window downloads are never captured, and you can
  set a minimum file size. Captured downloads are fetched again by Nexa without
  the browser's cookies, so leave capturing off for sites where downloads need
  a signed-in session.
- English and Persian (right-to-left) interface.

## Install

The Nexa installer registers the native messaging host
(`com.nexa.downloadmanager`) for every supported browser automatically. Nexa
must be running for downloads to be added. If the extension says "Nexa is not
running or not installed", start Nexa, or register the host again with
`nexa-native-host --register`.

### Chrome, Edge, Brave, Vivaldi

1. Open `chrome://extensions` (`edge://extensions`, `brave://extensions`, ...).
2. Turn on **Developer mode**.
3. Click **Load unpacked** and select this `browser-extension` folder.

The manifest contains a public key, so the extension ID is always
`ndlafmjbcbcjmkegfelbhgknmajgdbna`. That ID is allowed by the native host
out of the box.

### Firefox

1. Open `about:debugging`, then **This Firefox**.
2. Click **Load Temporary Add-on...** and select `browser-extension/manifest.json`.

Temporary add-ons are removed when Firefox restarts. For a permanent install
the add-on must be signed by [addons.mozilla.org](https://addons.mozilla.org)
(AMO). The add-on ID is `nexa@nexa-download-manager.app`, which the native host
allows by default.

### Using a different or modified extension

The native host only answers extensions it knows. If you publish a fork, load
the extension without the `key` field (which gives it a different ID), or use a
different Firefox add-on ID, add that ID in Nexa under
**Settings → Browser integration → allowed extension IDs** (Chromium IDs and
Firefox add-on IDs are listed separately). Browser integration must also be
turned on there; otherwise every request except `ping` is answered with
`unauthorized`.

## Protocol

Each request is one JSON message sent with `runtime.sendNativeMessage`:

```json
{ "version": 1, "type": "add_download", "id": "c0ffee", "payload": { "url": "https://example.com/file.zip", "referrer": "https://example.com/" } }
```

| Type | Payload | Used by |
| --- | --- | --- |
| `ping` | none | popup connection status |
| `add_download` | `url`, `filename?`, `referrer?` | link, media and page menus, "Send current page", captured downloads |
| `add_multiple_downloads` | `items: [{url, filename?, referrer?}]`, 1 to 200 items | selection menu, "Download all links on this page" (sent in batches) |
| `get_status` | `download_id?` | popup download list |
| `pause`, `resume`, `cancel` | `download_id` | popup download controls |

Responses are `{"version":1,"id":...,"success":true,...}` or
`{"version":1,"id":...,"success":false,"error":{"code":...,"message":...}}`. Error
codes are `invalid_json`, `message_too_large`, `unsupported_version`,
`unknown_type`, `invalid_payload`, `invalid_url`, `not_found`,
`app_unavailable`, `unauthorized` and `internal`. The extension only sends
`http`, `https` and `magnet` URLs. The full specification is in
[../docs/BROWSER_PROTOCOL.md](../docs/BROWSER_PROTOCOL.md).

## Privacy

No data leaves your computer. The extension makes no network requests, has no
analytics and loads no remote code. It talks only to the Nexa app on the same
machine, and only sends the URLs you choose (plus the page they came from as the
referrer). With capturing turned on, it also sends the URLs of the downloads you
start in the browser. Link collection reads a page only when you click a menu
item or a popup button. Options are stored in the browser's local extension
storage.

## Files

| File | Purpose |
| --- | --- |
| `manifest.json` | Manifest V3 for Chromium and Firefox |
| `background.js` | Context menus, notifications, download capture |
| `popup.html`, `popup.js`, `popup.css` | Toolbar popup |
| `native.js` | Native Messaging client (`send`, `sendMany`) |
| `collect.js` | Reads links from the page or the selection |
| `settings.js` | Stored options |
| `lib.js` | Pure helpers (URL checks, filtering, batching) |
| `_locales/` | English and Persian strings |
| `test/` | Unit tests: `node --test browser-extension/test/` |
