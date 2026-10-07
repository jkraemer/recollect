//! SQLite storage: one database file that several processes (the CLI, a sync daemon) open.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::error::{Error, Result};

mod candidates;
mod chunks;
mod memories;
mod peers;
mod queries;
mod schema;
mod sync;
#[cfg(test)]
pub(crate) mod test_support;

pub use memories::ImportCounts;
pub use peers::Peer;
pub use schema::SCHEMA_VERSION;
pub use sync::SyncApplied;

/// An open recollect database, migrated to the current schema.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (creating it and its directory if needed) the database file.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::file(parent))?;
        }
        register_sqlite_vec();
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        enable_write_ahead_logging(&conn, path)?;
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

    /// Begins a transaction that holds the write lock from the start, waiting
    /// up to the busy timeout for it. A deferred transaction reads first (the
    /// statement's virtual tables load their configuration) and would then
    /// have to upgrade to a write; SQLite fails such an upgrade with
    /// `SQLITE_BUSY` at once, without consulting the busy timeout.
    fn write_transaction(&mut self) -> Result<Transaction<'_>> {
        Ok(self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?)
    }
}

/// Puts the database file into WAL mode. The switch needs exclusive access and
/// SQLite fails it with `SQLITE_BUSY` at once if another connection is
/// reading the file, without consulting the busy timeout. Processes opening a
/// new database therefore take turns through a lock file; once the first one
/// has switched, the others find the file already in WAL mode.
fn enable_write_ahead_logging(conn: &Connection, path: &Path) -> Result<()> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(Error::file(&lock_path))?;
    lock.lock().map_err(Error::file(&lock_path))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    Ok(())
}

/// A `LIMIT` parameter; limits beyond SQLite's integer range mean no limit.
fn sql_limit(limit: usize) -> Value {
    Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX))
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    use crate::db::test_support::concurrently;
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
    fn a_data_directory_that_cannot_be_created_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let err = Database::open(&file.join("memories.db")).unwrap_err();
        assert!(
            matches!(&err, Error::File { path, .. } if *path == file),
            "{err}"
        );
    }

    #[test]
    fn a_lock_file_that_cannot_be_opened_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("memories.db.lock");
        std::fs::create_dir(&lock).unwrap();
        let err = Database::open(&dir.path().join("memories.db")).unwrap_err();
        assert!(
            matches!(&err, Error::File { path, .. } if *path == lock),
            "{err}"
        );
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
    fn a_fresh_database_can_be_opened_by_several_openers_at_once() {
        let dir = tempfile::tempdir().unwrap();
        for round in 0..10 {
            let path = dir.path().join(format!("memories-{round}.db"));
            for result in concurrently(8, |_| Database::open(&path)) {
                if let Err(err) = result {
                    panic!("a concurrent open failed: {err}");
                }
            }
        }
    }

    #[test]
    fn write_transactions_hold_the_write_lock_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.db");
        let mut db = Database::open(&path).unwrap();
        let _write = db.write_transaction().unwrap();
        let rival = Connection::open(&path).unwrap();
        rival.busy_timeout(Duration::ZERO).unwrap();
        let err = rival.execute_batch("BEGIN IMMEDIATE").unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
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
