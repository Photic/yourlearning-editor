use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug)]
pub struct SqliteState {
    db: Mutex<Pool<SqliteConnectionManager>>,
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
