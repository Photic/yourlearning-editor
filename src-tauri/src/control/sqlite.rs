use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug)]
pub struct SqliteState {
    db: Mutex<Pool<SqliteConnectionManager>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub hours: i64,
    pub minutes: i64,
    pub date: String,
    pub added_at: String,
}

impl SqliteState {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let manager = SqliteConnectionManager::file(path);
        let db = Pool::new(manager).map_err(|e| e.to_string())?;
        let conn = db.get().map_err(|e| e.to_string())?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
            [],
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS learning_history (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                url      TEXT NOT NULL,
                title    TEXT NOT NULL,
                hours    INTEGER NOT NULL DEFAULT 0,
                minutes  INTEGER NOT NULL DEFAULT 0,
                date     TEXT NOT NULL,
                added_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )
        .map_err(|e| e.to_string())?;

        Ok(Self { db: Mutex::new(db) })
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let conn = db.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([key]).map_err(|e| e.to_string())?;

        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => row.get(0).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    pub fn add_history(&self, url: &str, title: &str, hours: u64, minutes: u64, date: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let conn = db.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO learning_history (url, title, hours, minutes, date) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![url, title, hours as i64, minutes as i64, date],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Returns the most recent `limit` history entries, newest first.
    pub fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>, String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let conn = db.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, url, title, hours, minutes, date, added_at
                 FROM learning_history
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let entries = stmt
            .query_map([limit as i64], |row| {
                Ok(HistoryEntry {
                    id:       row.get(0)?,
                    url:      row.get(1)?,
                    title:    row.get(2)?,
                    hours:    row.get(3)?,
                    minutes:  row.get(4)?,
                    date:     row.get(5)?,
                    added_at: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(entries)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let conn = db.get().map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    }
}
