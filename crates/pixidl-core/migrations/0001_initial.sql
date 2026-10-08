-- pixidl — initial schema.

CREATE TABLE downloads (
    id              TEXT PRIMARY KEY NOT NULL,
    url             TEXT NOT NULL,
    original_url    TEXT NOT NULL,
    referrer        TEXT,
    filename        TEXT NOT NULL,
    save_dir        TEXT NOT NULL,
    category        TEXT NOT NULL DEFAULT 'General',
    engine          TEXT NOT NULL CHECK (engine IN ('http', 'torrent', 'video')),
    status          TEXT NOT NULL CHECK (status IN ('queued', 'preparing', 'downloading', 'paused', 'completed', 'failed', 'cancelled')),
    priority        INTEGER NOT NULL DEFAULT 1,
    queue_position  INTEGER NOT NULL DEFAULT 0,
    total_bytes     INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    speed_bps       INTEGER NOT NULL DEFAULT 0,
    upload_bps      INTEGER NOT NULL DEFAULT 0,
    eta_seconds     INTEGER,
    resumable       INTEGER,
    speed_limit_bps INTEGER,
    connections     INTEGER NOT NULL DEFAULT 1,
    error_kind      TEXT,
    error_message   TEXT,
    error_detail    TEXT,
    retry_count     INTEGER NOT NULL DEFAULT 0,
    engine_options  TEXT NOT NULL DEFAULT '{}',
    peers           INTEGER,
    seeds           INTEGER,
    info_hash       TEXT,
    title           TEXT,
    thumbnail       TEXT,
    -- HTTP validators used to make sure a resumed file is the same resource.
    etag            TEXT,
    last_modified   TEXT,
    -- Torrent source (.torrent bytes) when added from a file.
    torrent_data    BLOB,
    created_at      TEXT NOT NULL,
    started_at      TEXT,
    completed_at    TEXT,
    updated_at      TEXT NOT NULL,
    scheduled_at    TEXT,
    file_missing    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_downloads_status ON downloads(status);
CREATE INDEX idx_downloads_queue ON downloads(priority DESC, queue_position ASC);
CREATE INDEX idx_downloads_created ON downloads(created_at);

-- Byte ranges of a multi-connection HTTP download, used to resume each segment.
CREATE TABLE http_segments (
    download_id TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    start       INTEGER NOT NULL,
    end_incl    INTEGER NOT NULL,
    downloaded  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (download_id, idx)
);

-- Audit trail of state changes and errors (bounded per download by the app).
CREATE TABLE download_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    download_id TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
    at          TEXT NOT NULL,
    kind        TEXT NOT NULL,
    message     TEXT
);
CREATE INDEX idx_events_download ON download_events(download_id);

CREATE TABLE categories (
    name        TEXT PRIMARY KEY NOT NULL,
    extensions  TEXT NOT NULL DEFAULT '',
    subfolder   TEXT NOT NULL DEFAULT '',
    builtin     INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0
);

INSERT INTO categories (name, extensions, subfolder, builtin, sort_order) VALUES
    ('General',   '',                                             'General',   1, 0),
    ('Videos',    'mp4 mkv webm avi mov wmv flv m4v mpg mpeg 3gp', 'Videos',    1, 1),
    ('Music',     'mp3 flac wav aac ogg opus m4a wma',             'Music',     1, 2),
    ('Documents', 'pdf doc docx xls xlsx ppt pptx odt ods txt rtf epub csv', 'Documents', 1, 3),
    ('Programs',  'exe msi msix appx dmg pkg deb rpm appimage apk iso', 'Programs', 1, 4),
    ('Archives',  'zip rar 7z tar gz bz2 xz zst cab',              'Archives',  1, 5),
    ('Torrents',  'torrent',                                       'Torrents',  1, 6),
    ('Other',     '',                                              'Other',     1, 7);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
