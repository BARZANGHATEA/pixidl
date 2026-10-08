# Browser integration protocol (version 1)

Browser extensions talk to pixidl through **Native Messaging**.
The browser starts `pixidl-native-host(.exe)`; the host validates every message and
forwards it to the running app over a local, authenticated bridge. Nothing is ever
exposed to the network.

```
Extension ──native messaging (stdio)──► pixidl-native-host ──127.0.0.1 TCP + token──► pixidl app
```

## Host registration

| Item | Value |
|---|---|
| Host name | `com.pixidl.app` |
| Reference Chromium extension ID | `ndlafmjbcbcjmkegfelbhgknmajgdbna` |
| Reference Firefox add-on ID | `pixidl@pixidl.app` |

The installer runs `pixidl-native-host --register`, and the app re-registers on every
start, so manifests always point at the installed host. On Windows the manifests
live in `%APPDATA%\com.pixidl.app\native-messaging\` and are registered
under `HKCU\Software\{Google\Chrome, Microsoft\Edge, BraveSoftware\Brave-Browser,
Chromium, Mozilla}\NativeMessagingHosts\com.pixidl.app`. On Linux/macOS
they are written to each browser's per-user `NativeMessagingHosts` folder.

To let **your own extension** use the protocol, add its ID in
*Settings → Browser integration → Additional Chromium extension IDs* (or Firefox
add-on IDs). The manifests are regenerated immediately.

Maintenance commands: `pixidl-native-host --register [--extension-id ID]... [--firefox-id ID]...`,
`--unregister`, `--status` (JSON), `--version`.

## Framing

Standard native messaging: each message is a 32-bit length in native byte order
followed by UTF-8 JSON. Messages larger than **1 MiB** are rejected in both
directions.

## Request

```json
{ "version": 1, "type": "add_download", "id": "optional-correlation-id", "payload": { } }
```

- `version` — must be `1`. Any other value returns `unsupported_version`.
- `id` — optional string (≤128 chars) or number; echoed in the response.
- `payload` — object; may be omitted for `ping` and `get_status`.

## Response

```json
{ "version": 1, "id": "…", "success": true, "...": "…" }
{ "version": 1, "id": "…", "success": false, "error": { "code": "invalid_url", "message": "…" } }
```

## Message types

| Type | Payload | Success fields |
|---|---|---|
| `ping` | — | `app`, `app_version`, `protocol_version` |
| `add_download` | `url` (required), `filename`?, `referrer`? | `download_id`, `filename`, `engine` |
| `add_multiple_downloads` | `items`: 1–200 × `{url, filename?, referrer?}` | `added`, `results[]` (`url`, `success`, `download_id` or `error`) |
| `get_status` | `download_id`? | with id: `download`; without: `active`, `queued`, `download_bytes_per_second`, `downloads[]` (≤50) |
| `pause` / `resume` / `cancel` | `download_id` | `download_id` |

A download view contains `download_id`, `filename`, `status`, `downloaded_bytes`,
`total_bytes`, `speed_bytes_per_second`, `eta_seconds`, `error`. Local paths are never
returned.

Status values: `queued`, `preparing`, `downloading`, `paused`, `completed`, `failed`, `cancelled`.

## Validation and security rules

- URLs must be `http(s)://` with a host, or `magnet:?` with an info hash; max 16 KiB.
  `file:`, `javascript:`, `data:` etc. are rejected (`invalid_url`).
- `filename` is reduced to a single safe file name (path components, reserved
  Windows names and control/bidi characters removed). The browser **cannot choose a
  folder**: downloads always go to the default download folder.
- `referrer` is kept only if it is an http(s) URL. Cookies and credentials are never
  accepted or stored.
- If browser integration is disabled in Settings, every request except `ping`
  returns `unauthorized`.
- The same URL already queued or downloading is not added twice; its existing id is returned.

## Error codes

`invalid_json`, `message_too_large`, `unsupported_version`, `unknown_type`,
`invalid_payload`, `invalid_url`, `not_found`, `app_unavailable` (the app is not
running and could not be started), `unauthorized`, `internal`.

## The local bridge (app side)

- Listens on `127.0.0.1` on a random port; non-loopback peers are dropped.
- A fresh 256-bit random token is generated each launch and written with the port to
  `bridge.json` in the user's private data folder (mode 0600 on Unix). The first
  frame of every connection must be `{"auth":"<token>"}`; it is compared in constant
  time. A web page cannot read the token, and an HTTP request fails authentication.
- Frames are a 32-bit little-endian length + JSON (≤1 MiB); at most 16 concurrent
  connections; idle connections close after 60 s.
- If the app is not running, the host starts it in the background (`--background`)
  and waits up to 20 s.

## Example

```json
→ {"version":1,"type":"add_multiple_downloads","id":"7","payload":{"items":[
     {"url":"https://example.com/a.zip","referrer":"https://example.com/"},
     {"url":"magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"}]}}
← {"version":1,"id":"7","success":true,"added":2,"results":[
     {"url":"https://example.com/a.zip","success":true,"download_id":"…"},
     {"url":"magnet:?xt=…","success":true,"download_id":"…"}]}
```

The protocol is covered by `crates/pixidl-core/src/protocol.rs` unit tests and the
end-to-end test `crates/pixidl-native-host/tests/native_messaging.rs`, which drives the
real host binary.
