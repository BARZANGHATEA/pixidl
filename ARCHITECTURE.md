# Architecture

```
┌──────────────────────────── pixidl (one process) ────────────────────────────┐
│                                                                                              │
│  React UI (src/)  ──typed invoke()──►  Tauri shell (src-tauri/)  ──►  pixidl-core              │
│      ▲                                   commands.rs, tray, notifications,   ┌─────────────┐ │
│      └────── "pixidl://event" ManagerEvents ◄── clipboard, power, logging       │ Download    │ │
│                                                                               │ Manager     │ │
│                                                                               │  queue      │ │
│                                                                               │  scheduler  │ │
│   Loopback bridge (127.0.0.1, token) ◄────────────────────────────────────── │  recovery   │ │
│                                                                               └──┬───┬───┬──┘ │
│                                                                    HttpEngine ◄──┘   │   └──► VideoEngine (yt-dlp process)
│                                                                    TorrentEngine ◄───┘        │
│                                                     SQLite (pixidl.db) ◄── persistence         │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
        ▲
        │ length-prefixed JSON + auth token
pixidl-native-host(.exe)  ◄──native messaging (stdio)──  browser extension
```

## Crates and folders

| Path | Responsibility |
|---|---|
| `crates/pixidl-core` | Everything that does not need a window: types and state machine, SQLite + migrations, settings, URL detection, the three engines, the queue manager, security helpers, the browser protocol and the loopback bridge. Fully testable headless. |
| `crates/pixidl-native-host` | Small binary started by browsers. Validates messages, starts the app if needed, forwards over the bridge. Also `--register/--unregister/--status`. |
| `src-tauri` | Desktop shell: IPC commands, event forwarding, tray, notifications, single instance + file association, autostart, clipboard monitor, after-queue power action, logging. |
| `src/` | React UI. `services/api.ts` is the only place that calls `invoke`; `services/events.ts` subscribes to backend events; state lives in zustand stores. |
| `browser-extension/` | Reference MV3 extension for Chromium and Firefox. |

## Download flow

```
URL ─► detector::detect ─► (optional) inspect_url ─► AddDownloadRequest
    ─► DownloadManager::add  (validate, choose engine, approved folder, safe unique name, category)
    ─► SQLite row (status = queued) ─► schedule_pass picks it when a slot is free
    ─► Engine::run(JobContext) ─► ProgressCell (sampled every 500 ms) + MetaUpdate channel
    ─► finish_job ─► completed | paused | queued (retry/back-off) | failed | cancelled
```

### Engine abstraction

`engines::Engine` has two methods:

- `run(JobContext) -> Result<EngineOutcome>` performs or resumes a download. It
  returns `Completed { filename }` or `Stopped`, or a classified `DownloadError`.
- `cleanup(&Download, delete_completed)` removes partial data after a cancel or a
  removal.

`JobContext` carries:

- a snapshot of the download and of the settings
- the database handle and the shared HTTP client
- a `watch` channel for **Run/Pause/Cancel**
- the progress cell and the metadata channel
- the per-download and global rate limiters

The manager never knows *how* a download is performed. The user, or the detector,
chooses the engine.

### State machine

`DownloadStatus::can_transition_to` defines every allowed transition. All status
changes go through `Inner::transition`, which rejects anything else:

```
queued ─► preparing ─► downloading ─► completed
  │ ▲         │            │
  │ └─────────┴────────────┤   (requeue: scheduler stop, retry back-off, shutdown)
  ▼                        ├─► paused ─► queued
paused                     ├─► failed ─► queued (retry)
                           └─► cancelled ─► queued (restart)
```

### HTTP engine

**Probing.** Sends `Range: bytes=N-`, plus `If-Range` with a strong ETag or
`Last-Modified` when resuming.

- `206` → resumable. The total comes from `Content-Range`.
- `200` while resuming → the server ignored the range or the file changed. The
  engine restarts from zero and logs an event saying so.
- `416` at the full size → the file is already complete.

**Single connection.** Streams into `<name>.part` through a 512 KiB buffer. Resume
uses the actual `.part` size on disk.

**Multi-connection.** Used for resumable files of 2 MiB or more. The engine:

- pre-allocates the `.part` file
- splits it into N segments (`plan_segments`) and starts N workers; the response
  to the first request is reused for the range it starts at
- work stealing (`SegmentTable`): a worker takes the lowest unowned range; when
  none is left it splits the active range with the most bytes remaining at the
  midpoint of what remains (only if both halves get at least `MIN_SPLIT`,
  512 KiB) and downloads the upper half. Bytes are claimed under the table
  lock, so the victim stops exactly at its new end and closes its connection.
  All connections stay busy until the last MiB instead of the tail of the file
  running on one connection
- connection limits: a worker whose request is refused before any data (429,
  503, 403, 200 to a range request, refused/reset connection) while another
  worker is receiving gives its range back and exits; the download continues
  with fewer connections and logs "Server allows only N connections" once.
  Other errors are retried per range with backoff; the download fails only
  when no worker is left to make progress
- persists the committed bytes per segment every 2 s and after every split
  (`http_segments` table, replaced as a whole), so a crash loses at most the
  unflushed buffers; on resume, finished segments are merged into their
  successor and only uncommitted bytes are requested again
- publishes a live segment map (`SegmentView`, command `get_segments`) shown in
  the details drawer
- uses HTTP/1.1 only, so every range really gets its own TCP connection
  (HTTP/2 would multiplex them onto one)

**Rate limiting.** Every chunk takes tokens from the per-download limiter and then
from the global token bucket (`ratelimit.rs`). Waiting for tokens can be cancelled.

**Finalising.** The engine verifies the byte count, then atomically renames `.part`
to the final name. Duplicate names follow the duplicate policy (`file (1).zip`).

**Errors.** Errors are classified into `ErrorKind`: network, timeout, server
rejected, not found, permission, disk full, resume not supported, and so on. Only
transient ones are retried automatically, using `RetryPolicy` (exponential
back-off).

### Torrent engine

The engine wraps a lazily started librqbit `Session`. Persistence and fastresume
live under `<data>/torrent`.

1. **Metadata.** Metadata is resolved with a list-only add, which uses DHT and
   trackers for magnets. The resulting `.torrent` bytes are stored in the database,
   so a restart never has to resolve the magnet again.
2. **Add.** The torrent is then added with the chosen file selection and output
   folder.
3. **Stats.** Stats are polled every 500 ms. Only values the engine exposes are
   shown: progress, download/upload speed, ETA and live peers. librqbit does not
   report seeds separately, so seeds are never displayed.
4. **Pause and cancel.** Pause uses `session.pause`. Cancel deletes the torrent and
   its files.

Torrent sessions restored from persistence are paused at startup, so the queue
alone decides what runs.

### Video engine

Runs yt-dlp as a child process with an argument array:

- `--ignore-config`, `--no-playlist`, `-f <selector>`
- `-P <dir>` for the destination and `-P temp:<dir>/.pixidl-partial/<id>` for partial files
- a machine-readable `--progress-template`
- `--print after_move:filepath` to report the final file
- the URL is always placed after `--`

The engine accepts only an output file that lands inside the destination folder.

- **Pause** stops the process. **Resume** runs the same command again; yt-dlp
  continues its `.part` files.
- **Cancel** deletes the per-download temp folder.
- **Speed limit.** yt-dlp's `--limit-rate` gets the stricter of the per-download
  and global limits when the download starts.

### Manager loops

- **Queue loop.** Wakes on `Notify` or every second. It enforces
  `max_concurrent_downloads`, priority and queue order, `scheduled_at`, retry
  back-off and the global time window. Outside the window, running downloads go
  back to the queue.
- **Progress loop.** Every 500 ms it samples each job's `ProgressCell` and emits
  **one** batched `download_progress` event for the downloads that changed. Every
  3 s it writes progress to SQLite.
- **Recovery.** At startup, downloads left in `preparing`/`downloading` move to
  `queued`, or to `paused` if auto-resume is off. Their byte count is recomputed
  from the bytes actually on disk. Completed downloads whose file is gone get
  `file_missing`.
- **Shutdown.** Pauses every job with the "requeue" intent and waits up to 8 s for
  files to flush. The jobs resume on the next launch.
- **Queue finished.** `QueueFinished` is only emitted after at least one completion
  and with nothing running or waiting. This guards the optional after-queue
  sleep/shutdown action, which also shows a cancellable 60-second countdown.

## Persistence

SQLite in WAL mode, with migrations in `crates/pixidl-core/migrations/` (applied by
`rusqlite_migration`; `PRAGMA user_version` tracks the schema version).

| Table | Contents |
|---|---|
| `downloads` | one row per download: URLs, file name, folder, category, engine, status, priority and queue position, sizes and speeds, error kind/message/detail, retry count, engine options, torrent data, HTTP validators, timestamps |
| `http_segments` | per-segment progress for multi-connection resume |
| `download_events` | bounded activity log per download, shown under *Details* |
| `categories` | built-in and custom categories, with their extensions and subfolders |
| `settings` | one JSON value per key, so new settings need no migration and invalid values fall back to defaults |

## Security model

- **URLs.** All URLs are validated (`security::validate_url`): http, https or
  magnet only, with a length cap and no control characters.
- **File names.** Names are sanitised: path components, reserved Windows names,
  invalid characters, bidi overrides, trailing dots and over-long names are removed
  or replaced. Joined paths are checked to stay inside the destination folder.
- **Destinations.** Downloads can only be written inside approved folders: the
  default folder, or folders the user picked in a system dialog. The browser can
  never choose a folder.
- **External processes.** They are started with argument arrays, never through a
  shell, and without a console window on Windows.
- **Browser bridge.**
  - It listens only on loopback and requires a per-launch 256-bit token stored in
    the user's data folder.
  - Messages are size-capped and validated twice: in the host and in the app.
  - The web layer has no filesystem permissions. All file operations go through
    typed commands, and the CSP blocks remote scripts.
- **Credentials.** No cookies or credentials are stored. Proxy credentials are
  stripped, and query strings and credentials are removed from URLs before logging.
- **No auto-run.** Downloaded files are never executed automatically. "Open file"
  is an explicit user action.

## Frontend

React 18 + TypeScript + Vite.

- **Styling.** Plain CSS with design tokens (`src/styles/app.css`), light/dark
  themes and a custom accent. Logical properties make RTL layouts work.
- **Icons and fonts.** lucide-react icons. Inter and Vazirmatn are bundled
  locally.
- **Translations.** i18next with `en.json` and `fa.json`. A test enforces that both
  files have the same keys and placeholders, and that every key used in the code
  exists.
- **Types.** Generated from Rust by ts-rs into `src/types/generated`.
