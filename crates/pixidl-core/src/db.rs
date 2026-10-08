//! SQLite persistence with versioned migrations.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{DownloadError, Result};
use crate::settings::Settings;
use crate::types::*;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../migrations/0001_initial.sql"))])
}

/// One segment of a multi-connection HTTP download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub idx: u32,
    pub start: u64,
    pub end_incl: u64,
    pub downloaded: u64,
}

impl Segment {
    pub fn len(&self) -> u64 {
        self.end_incl - self.start + 1
    }
    pub fn is_empty(&self) -> bool {
        self.end_incl < self.start
    }
    pub fn is_done(&self) -> bool {
        self.downloaded >= self.len()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Category {
    pub name: String,
    /// Space-separated lower-case extensions.
    pub extensions: String,
    pub subfolder: String,
    pub builtin: bool,
    #[ts(type = "number")]
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadEvent {
    pub at: String,
    pub kind: String,
    pub message: Option<String>,
}

/// HTTP validators for safe resume.
#[derive(Debug, Clone, Default)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

const COLUMNS: &str = "id, url, original_url, referrer, filename, save_dir, category, engine, status, priority, \
    queue_position, total_bytes, downloaded_bytes, speed_bps, upload_bps, eta_seconds, resumable, speed_limit_bps, \
    connections, error_kind, error_message, error_detail, retry_count, engine_options, peers, seeds, info_hash, \
    title, thumbnail, created_at, started_at, completed_at, updated_at, scheduled_at, file_missing";

fn row_to_download(r: &Row<'_>) -> rusqlite::Result<Download> {
    let engine: String = r.get(7)?;
    let status: String = r.get(8)?;
    let opts: String = r.get(23)?;
    let ek: Option<String> = r.get(19)?;
    Ok(Download {
        id: r.get(0)?,
        url: r.get(1)?,
        original_url: r.get(2)?,
        referrer: r.get(3)?,
        filename: r.get(4)?,
        save_dir: r.get(5)?,
        category: r.get(6)?,
        engine: EngineKind::parse(&engine).unwrap_or(EngineKind::Http),
        status: DownloadStatus::parse(&status).unwrap_or(DownloadStatus::Failed),
        priority: Priority::from_i64(r.get(9)?),
        queue_position: r.get(10)?,
        total_bytes: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
        downloaded_bytes: r.get::<_, i64>(12)? as u64,
        speed_bps: r.get::<_, i64>(13)? as u64,
        upload_bps: r.get::<_, i64>(14)? as u64,
        eta_seconds: r.get::<_, Option<i64>>(15)?.map(|v| v as u64),
        resumable: r.get(16)?,
        speed_limit_bps: r.get::<_, Option<i64>>(17)?.map(|v| v as u64),
        connections: r.get::<_, i64>(18)? as u32,
        error_kind: ek.map(|s| ErrorKind::parse(&s)),
        error_message: r.get(20)?,
        error_detail: r.get(21)?,
        retry_count: r.get::<_, i64>(22)? as u32,
        engine_options: serde_json::from_str(&opts).unwrap_or_default(),
        peers: r.get::<_, Option<i64>>(24)?.map(|v| v as u32),
        seeds: r.get::<_, Option<i64>>(25)?.map(|v| v as u32),
        info_hash: r.get(26)?,
        title: r.get(27)?,
        thumbnail: r.get(28)?,
        created_at: r.get(29)?,
        started_at: r.get(30)?,
        completed_at: r.get(31)?,
        updated_at: r.get(32)?,
        scheduled_at: r.get(33)?,
        file_missing: r.get::<_, i64>(34)? != 0,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DownloadError::fs("Cannot create data folder", &e))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations()
            .to_latest(&mut conn)
            .map_err(|e| DownloadError::new(ErrorKind::Unknown, "Database migration failed").with_detail(e.to_string()))?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.lock().pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    // ---------------------------------------------------------------- downloads

    pub fn insert_download(&self, d: &Download) -> Result<()> {
        let c = self.conn.lock();
        c.execute(
            &format!("INSERT INTO downloads ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35)"),
            params![
                d.id, d.url, d.original_url, d.referrer, d.filename, d.save_dir, d.category,
                d.engine.as_str(), d.status.as_str(), d.priority.as_i64(), d.queue_position,
                d.total_bytes.map(|v| v as i64), d.downloaded_bytes as i64, d.speed_bps as i64,
                d.upload_bps as i64, d.eta_seconds.map(|v| v as i64), d.resumable,
                d.speed_limit_bps.map(|v| v as i64), d.connections as i64,
                d.error_kind.map(|k| k.as_str()), d.error_message, d.error_detail, d.retry_count as i64,
                serde_json::to_string(&d.engine_options).unwrap_or_else(|_| "{}".into()),
                d.peers.map(|v| v as i64), d.seeds.map(|v| v as i64), d.info_hash, d.title, d.thumbnail,
                d.created_at, d.started_at, d.completed_at, d.updated_at, d.scheduled_at, d.file_missing as i64,
            ],
        )?;
        Ok(())
    }

    /// Writes every mutable column of a download.
    pub fn update_download(&self, d: &Download) -> Result<()> {
        let c = self.conn.lock();
        c.execute(
            "UPDATE downloads SET url=?2, filename=?3, save_dir=?4, category=?5, engine=?6, status=?7, priority=?8,
             queue_position=?9, total_bytes=?10, downloaded_bytes=?11, speed_bps=?12, upload_bps=?13, eta_seconds=?14,
             resumable=?15, speed_limit_bps=?16, connections=?17, error_kind=?18, error_message=?19, error_detail=?20,
             retry_count=?21, engine_options=?22, peers=?23, seeds=?24, info_hash=?25, title=?26, thumbnail=?27,
             started_at=?28, completed_at=?29, updated_at=?30, scheduled_at=?31, file_missing=?32, referrer=?33
             WHERE id=?1",
            params![
                d.id, d.url, d.filename, d.save_dir, d.category, d.engine.as_str(), d.status.as_str(),
                d.priority.as_i64(), d.queue_position, d.total_bytes.map(|v| v as i64), d.downloaded_bytes as i64,
                d.speed_bps as i64, d.upload_bps as i64, d.eta_seconds.map(|v| v as i64), d.resumable,
                d.speed_limit_bps.map(|v| v as i64), d.connections as i64, d.error_kind.map(|k| k.as_str()),
                d.error_message, d.error_detail, d.retry_count as i64,
                serde_json::to_string(&d.engine_options).unwrap_or_else(|_| "{}".into()),
                d.peers.map(|v| v as i64), d.seeds.map(|v| v as i64), d.info_hash, d.title, d.thumbnail,
                d.started_at, d.completed_at, d.updated_at, d.scheduled_at, d.file_missing as i64, d.referrer,
            ],
        )?;
        Ok(())
    }

    /// Cheap progress-only update used by the periodic flush.
    pub fn update_progress(&self, id: &str, downloaded: u64, total: Option<u64>) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE downloads SET downloaded_bytes=?2, total_bytes=COALESCE(?3, total_bytes), updated_at=?4 WHERE id=?1",
            params![id, downloaded as i64, total.map(|v| v as i64), now()],
        )?;
        Ok(())
    }

    pub fn get_download(&self, id: &str) -> Result<Option<Download>> {
        let c = self.conn.lock();
        Ok(c.query_row(&format!("SELECT {COLUMNS} FROM downloads WHERE id=?1"), [id], row_to_download).optional()?)
    }

    pub fn list_downloads(&self) -> Result<Vec<Download>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM downloads ORDER BY created_at DESC"))?;
        let rows = st.query_map([], row_to_download)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Queued downloads in start order: priority, then queue position, then age.
    pub fn queued_in_order(&self) -> Result<Vec<Download>> {
        let c = self.conn.lock();
        let mut st = c.prepare(&format!(
            "SELECT {COLUMNS} FROM downloads WHERE status='queued' ORDER BY priority DESC, queue_position ASC, created_at ASC"
        ))?;
        let rows = st.query_map([], row_to_download)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn delete_download(&self, id: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM downloads WHERE id=?1", [id])? > 0)
    }

    /// Removes finished (completed/failed/cancelled) history rows. Files are untouched.
    pub fn clear_history(&self) -> Result<Vec<String>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT id FROM downloads WHERE status IN ('completed','failed','cancelled')")?;
        let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        c.execute("DELETE FROM downloads WHERE status IN ('completed','failed','cancelled')", [])?;
        Ok(ids)
    }

    pub fn next_queue_position(&self) -> Result<i64> {
        Ok(self.conn.lock().query_row("SELECT COALESCE(MAX(queue_position), 0) + 1 FROM downloads", [], |r| r.get(0))?)
    }

    pub fn set_queue_positions(&self, ordered_ids: &[String]) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        for (i, id) in ordered_ids.iter().enumerate() {
            tx.execute("UPDATE downloads SET queue_position=?2 WHERE id=?1", params![id, i as i64 + 1])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn filename_reserved(&self, save_dir: &str, filename: &str, except_id: Option<&str>) -> Result<bool> {
        let c = self.conn.lock();
        let n: i64 = c.query_row(
            "SELECT COUNT(*) FROM downloads WHERE save_dir=?1 AND filename=?2 AND status NOT IN ('cancelled','failed','completed') AND id != COALESCE(?3, '')",
            params![save_dir, filename, except_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn find_active_by_url(&self, url: &str) -> Result<Option<Download>> {
        let c = self.conn.lock();
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM downloads WHERE original_url=?1 AND status IN ('queued','preparing','downloading','paused') LIMIT 1"),
            [url],
            row_to_download,
        )
        .optional()?)
    }

    pub fn set_validators(&self, id: &str, v: &Validators) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE downloads SET etag=?2, last_modified=?3 WHERE id=?1",
            params![id, v.etag, v.last_modified],
        )?;
        Ok(())
    }

    pub fn validators(&self, id: &str) -> Result<Validators> {
        let c = self.conn.lock();
        Ok(c.query_row("SELECT etag, last_modified FROM downloads WHERE id=?1", [id], |r| {
            Ok(Validators { etag: r.get(0)?, last_modified: r.get(1)? })
        })
        .optional()?
        .unwrap_or_default())
    }

    pub fn set_torrent_data(&self, id: &str, data: &[u8]) -> Result<()> {
        self.conn.lock().execute("UPDATE downloads SET torrent_data=?2 WHERE id=?1", params![id, data])?;
        Ok(())
    }

    pub fn torrent_data(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let c = self.conn.lock();
        Ok(c.query_row("SELECT torrent_data FROM downloads WHERE id=?1", [id], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    // ---------------------------------------------------------------- segments

    pub fn save_segments(&self, id: &str, segs: &[Segment]) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM http_segments WHERE download_id=?1", [id])?;
        for s in segs {
            tx.execute(
                "INSERT INTO http_segments (download_id, idx, start, end_incl, downloaded) VALUES (?1,?2,?3,?4,?5)",
                params![id, s.idx, s.start as i64, s.end_incl as i64, s.downloaded as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn update_segment_progress(&self, id: &str, segs: &[(u32, u64)]) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        for (idx, downloaded) in segs {
            tx.execute(
                "UPDATE http_segments SET downloaded=?3 WHERE download_id=?1 AND idx=?2",
                params![id, idx, *downloaded as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_segments(&self, id: &str) -> Result<Vec<Segment>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT idx, start, end_incl, downloaded FROM http_segments WHERE download_id=?1 ORDER BY idx")?;
        let rows = st
            .query_map([id], |r| {
                Ok(Segment {
                    idx: r.get::<_, i64>(0)? as u32,
                    start: r.get::<_, i64>(1)? as u64,
                    end_incl: r.get::<_, i64>(2)? as u64,
                    downloaded: r.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn clear_segments(&self, id: &str) -> Result<()> {
        self.conn.lock().execute("DELETE FROM http_segments WHERE download_id=?1", [id])?;
        Ok(())
    }

    // ---------------------------------------------------------------- events

    pub fn add_event(&self, id: &str, kind: &str, message: Option<&str>) -> Result<()> {
        let c = self.conn.lock();
        c.execute(
            "INSERT INTO download_events (download_id, at, kind, message) VALUES (?1, ?2, ?3, ?4)",
            params![id, now(), kind, message],
        )?;
        // Keep the log bounded.
        c.execute(
            "DELETE FROM download_events WHERE download_id=?1 AND id NOT IN
             (SELECT id FROM download_events WHERE download_id=?1 ORDER BY id DESC LIMIT 100)",
            [id],
        )?;
        Ok(())
    }

    pub fn events(&self, id: &str) -> Result<Vec<DownloadEvent>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT at, kind, message FROM download_events WHERE download_id=?1 ORDER BY id ASC")?;
        let rows = st
            .query_map([id], |r| Ok(DownloadEvent { at: r.get(0)?, kind: r.get(1)?, message: r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---------------------------------------------------------------- categories

    pub fn categories(&self) -> Result<Vec<Category>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT name, extensions, subfolder, builtin, sort_order FROM categories ORDER BY sort_order, name")?;
        let rows = st
            .query_map([], |r| {
                Ok(Category {
                    name: r.get(0)?,
                    extensions: r.get(1)?,
                    subfolder: r.get(2)?,
                    builtin: r.get::<_, i64>(3)? != 0,
                    sort_order: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn upsert_category(&self, cat: &Category) -> Result<()> {
        let name = cat.name.trim();
        if name.is_empty() || name.len() > 64 {
            return Err(DownloadError::new(ErrorKind::Unknown, "Invalid category name"));
        }
        let ext: String = cat
            .extensions
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        let sub = crate::security::sanitize_filename(if cat.subfolder.trim().is_empty() { name } else { &cat.subfolder });
        self.conn.lock().execute(
            "INSERT INTO categories (name, extensions, subfolder, builtin, sort_order) VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(name) DO UPDATE SET extensions=excluded.extensions, subfolder=excluded.subfolder, sort_order=excluded.sort_order",
            params![name, ext, sub, cat.sort_order],
        )?;
        Ok(())
    }

    pub fn delete_category(&self, name: &str) -> Result<()> {
        let c = self.conn.lock();
        let builtin: Option<i64> = c.query_row("SELECT builtin FROM categories WHERE name=?1", [name], |r| r.get(0)).optional()?;
        if builtin == Some(1) {
            return Err(DownloadError::new(ErrorKind::Unknown, "Built-in categories cannot be deleted"));
        }
        c.execute("DELETE FROM categories WHERE name=?1", [name])?;
        c.execute("UPDATE downloads SET category='General' WHERE category=?1", [name])?;
        Ok(())
    }

    /// Category for a filename using the (user-customisable) category table,
    /// falling back to built-in rules.
    pub fn detect_category(&self, filename: &str, engine: EngineKind) -> String {
        if engine == EngineKind::Torrent {
            return "Torrents".into();
        }
        let ext = crate::security::extension_of(filename);
        let last_ext = ext.rsplit('.').next().unwrap_or("").to_string();
        if !ext.is_empty() {
            if let Ok(cats) = self.categories() {
                // Custom (non built-in) categories win over built-ins.
                let mut sorted = cats;
                sorted.sort_by_key(|c| c.builtin);
                for c in sorted {
                    if c.extensions.split(' ').any(|e| !e.is_empty() && (e == ext || e == last_ext)) {
                        return c.name;
                    }
                }
            }
        }
        crate::security::category_for(filename, engine).to_string()
    }

    // ---------------------------------------------------------------- key/value

    /// Small app state stored next to the settings (keys are prefixed so they
    /// never collide with settings).
    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        let c = self.conn.lock();
        Ok(c.query_row("SELECT value FROM settings WHERE key=?1", [format!("kv.{key}")], |r| r.get(0)).optional()?)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("kv.{key}"), value],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- settings

    pub fn load_settings(&self) -> Result<Settings> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT key, value FROM settings")?;
        let pairs = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Settings::from_pairs(pairs))
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        for (k, v) in s.to_pairs() {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(id: &str) -> Download {
        let t = now();
        Download {
            id: id.into(),
            url: "https://example.com/a.zip".into(),
            original_url: "https://example.com/a.zip".into(),
            referrer: None,
            filename: "a.zip".into(),
            save_dir: "/tmp".into(),
            category: "Archives".into(),
            engine: EngineKind::Http,
            status: DownloadStatus::Queued,
            priority: Priority::Normal,
            queue_position: 1,
            total_bytes: Some(100),
            downloaded_bytes: 0,
            speed_bps: 0,
            upload_bps: 0,
            eta_seconds: None,
            resumable: None,
            speed_limit_bps: None,
            connections: 1,
            error_kind: None,
            error_message: None,
            error_detail: None,
            retry_count: 0,
            engine_options: EngineOptions::default(),
            peers: None,
            seeds: None,
            info_hash: None,
            title: None,
            thumbnail: None,
            created_at: t.clone(),
            started_at: None,
            completed_at: None,
            updated_at: t,
            scheduled_at: None,
            file_missing: false,
        }
    }

    #[test]
    fn migrations_are_valid() {
        assert!(migrations().validate().is_ok());
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn download_crud() {
        let db = Db::open_in_memory().unwrap();
        let mut d = sample("1");
        d.engine_options.format_id = Some("137+140".into());
        db.insert_download(&d).unwrap();
        assert_eq!(db.get_download("1").unwrap().unwrap(), d);
        d.status = DownloadStatus::Downloading;
        d.downloaded_bytes = 50;
        d.error_kind = Some(ErrorKind::Timeout);
        db.update_download(&d).unwrap();
        assert_eq!(db.get_download("1").unwrap().unwrap(), d);
        db.update_progress("1", 75, Some(100)).unwrap();
        assert_eq!(db.get_download("1").unwrap().unwrap().downloaded_bytes, 75);
        assert!(db.delete_download("1").unwrap());
        assert!(db.get_download("1").unwrap().is_none());
    }

    #[test]
    fn queue_order() {
        let db = Db::open_in_memory().unwrap();
        for (id, prio, pos) in [("a", Priority::Normal, 2), ("b", Priority::Normal, 1), ("c", Priority::High, 3), ("d", Priority::Low, 0)] {
            let mut d = sample(id);
            d.priority = prio;
            d.queue_position = pos;
            db.insert_download(&d).unwrap();
        }
        let ids: Vec<_> = db.queued_in_order().unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["c", "b", "a", "d"]);
        db.set_queue_positions(&["a".into(), "b".into()]).unwrap();
        let ids: Vec<_> = db.queued_in_order().unwrap().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["c", "a", "b", "d"]);
    }

    #[test]
    fn segments_cascade() {
        let db = Db::open_in_memory().unwrap();
        db.insert_download(&sample("s")).unwrap();
        let segs = vec![Segment { idx: 0, start: 0, end_incl: 49, downloaded: 10 }, Segment { idx: 1, start: 50, end_incl: 99, downloaded: 0 }];
        db.save_segments("s", &segs).unwrap();
        db.update_segment_progress("s", &[(1, 25)]).unwrap();
        let back = db.load_segments("s").unwrap();
        assert_eq!(back[1].downloaded, 25);
        db.delete_download("s").unwrap();
        assert!(db.load_segments("s").unwrap().is_empty());
    }

    #[test]
    fn settings_persist() {
        let db = Db::open_in_memory().unwrap();
        let mut s = db.load_settings().unwrap();
        assert_eq!(s, { let mut d = Settings::default(); d.validate(); d });
        s.max_concurrent_downloads = 5;
        s.global_speed_limit_bps = Some(1_000_000);
        db.save_settings(&s).unwrap();
        assert_eq!(db.load_settings().unwrap(), s);
    }

    #[test]
    fn categories_and_detection() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.categories().unwrap().len() >= 8);
        assert_eq!(db.detect_category("x.mkv", EngineKind::Http), "Videos");
        db.upsert_category(&Category { name: "Books".into(), extensions: ".epub, mobi".into(), subfolder: String::new(), builtin: false, sort_order: 10 }).unwrap();
        assert_eq!(db.detect_category("novel.EPUB", EngineKind::Http), "Books");
        assert!(db.delete_category("Videos").is_err());
        db.delete_category("Books").unwrap();
        assert_eq!(db.detect_category("novel.epub", EngineKind::Http), "Documents");
    }

    #[test]
    fn history_clear_keeps_active() {
        let db = Db::open_in_memory().unwrap();
        let mut a = sample("a");
        a.status = DownloadStatus::Completed;
        db.insert_download(&a).unwrap();
        db.insert_download(&sample("b")).unwrap();
        assert_eq!(db.clear_history().unwrap(), vec!["a".to_string()]);
        assert_eq!(db.list_downloads().unwrap().len(), 1);
    }

    #[test]
    fn events_bounded() {
        let db = Db::open_in_memory().unwrap();
        db.insert_download(&sample("e")).unwrap();
        for i in 0..120 {
            db.add_event("e", "progress", Some(&i.to_string())).unwrap();
        }
        assert_eq!(db.events("e").unwrap().len(), 100);
    }
}
