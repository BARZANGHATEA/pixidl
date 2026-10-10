-- pixidl — user-created download queues.
--
-- Every download belongs to exactly one queue. The built-in "main" queue
-- cannot be deleted; its empty name means "use the translated default name".
-- A stopped queue (running = 0) starts nothing new.

CREATE TABLE queues (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    max_concurrent  INTEGER NOT NULL DEFAULT 2 CHECK (max_concurrent BETWEEN 1 AND 20),
    running         INTEGER NOT NULL DEFAULT 1,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);

-- The main queue only follows the global limit (max_concurrent_downloads) by default.
INSERT INTO queues (id, name, max_concurrent, running, sort_order, created_at)
VALUES ('main', '', 20, 1, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

-- Existing downloads join the main queue. (No REFERENCES clause: SQLite does not
-- allow adding a foreign-key column with a non-NULL default; the manager keeps
-- the column consistent and moves downloads to "main" when a queue is deleted.)
ALTER TABLE downloads ADD COLUMN queue_id TEXT NOT NULL DEFAULT 'main';
CREATE INDEX idx_downloads_queue_id ON downloads(queue_id);
