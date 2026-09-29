//! SQLite storage: one database file shared by the CLI and, later, the sync daemon.

use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::Result;

pub use schema::SCHEMA_VERSION;

/// An open recollect database, migrated to the current schema.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (creating it and its directory if needed) the database file.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        register_sqlite_vec();
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrated(conn)
    }

    /// A private in-memory database, for tests of the storage layer.
    pub fn open_in_memory() -> Result<Self> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrated(conn)
    }

    fn migrated(mut conn: Connection) -> Result<Self> {
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }
}

/// Serializes a vector the way sqlite-vec reads it: little-endian `f32`s.
pub(crate) fn embedding_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Makes sqlite-vec's functions available on every connection opened afterwards.
fn register_sqlite_vec() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        type EntryPoint = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::ffi::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::ffi::c_int;
        // SAFETY: sqlite3_vec_init is sqlite-vec's extension entry point, which
        // has exactly the EntryPoint signature SQLite calls auto extensions with.
        unsafe {
            let entry_point = std::mem::transmute::<*const (), EntryPoint>(
                sqlite_vec::sqlite3_vec_init as *const (),
            );
            rusqlite::ffi::sqlite3_auto_extension(Some(entry_point));
        }
    });
}

mod chunks;
mod memories;
mod queries;
mod schema;
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    use crate::error::Error;

    fn user_version(db: &Database) -> i64 {
        db.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn open_creates_the_file_and_its_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("memories.db");
        Database::open(&path).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn migrations_bring_a_new_database_to_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("memories.db")).unwrap();
        assert_eq!(user_version(&db), SCHEMA_VERSION);
    }

    #[test]
    fn reopening_a_current_database_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.db");
        drop(Database::open(&path).unwrap());
        let db = Database::open(&path).unwrap();
        assert_eq!(user_version(&db), SCHEMA_VERSION);
    }

    #[test]
    fn a_database_from_a_newer_binary_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let err = Database::open(&path).unwrap_err();
        assert!(
            matches!(err, Error::SchemaTooNew { found, supported } if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION),
            "{err}"
        );
    }

    #[test]
    fn file_databases_use_write_ahead_logging() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("memories.db")).unwrap();
        let mode: String = db
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn sqlite_vec_is_compiled_in() {
        let db = Database::open_in_memory().unwrap();
        let version: String = db
            .conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .unwrap();
        assert!(version.starts_with('v'), "{version}");
        let distance: f64 = db
            .conn
            .query_row(
                "SELECT vec_distance_cosine(?1, ?2)",
                params![embedding_blob(&[1.0, 0.0]), embedding_blob(&[0.0, 1.0])],
                |row| row.get(0),
            )
            .unwrap();
        assert!((distance - 1.0).abs() < 1e-6, "{distance}");
    }

    #[test]
    fn fts5_is_compiled_in() {
        let db = Database::open_in_memory().unwrap();
        db.conn
            .execute_batch("CREATE VIRTUAL TABLE probe USING fts5(body)")
            .unwrap();
    }
}
