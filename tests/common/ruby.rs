//! Ruby-shaped data directories for the migration tests, built from the
//! `memories` DDL of the Ruby server's three schema generations.

use std::path::Path;

use rusqlite::{Connection, params};

/// The newest Ruby schema: sync columns and `source`.
pub const WITH_SOURCE: &str = "CREATE TABLE memories (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  content TEXT NOT NULL,
  memory_type TEXT NOT NULL DEFAULT 'note',
  tags TEXT,
  metadata TEXT,
  embedding BLOB,
  created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  source TEXT DEFAULT 'unknown'
, global_id TEXT, origin_peer TEXT, deleted_at TEXT, deleted_by_peer TEXT)";

/// Sync columns without `source`, as in most Ruby files.
pub const WITHOUT_SOURCE: &str = "CREATE TABLE memories (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  content TEXT NOT NULL,
  memory_type TEXT NOT NULL DEFAULT 'note',
  tags TEXT,
  metadata TEXT,
  embedding BLOB,
  created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
, global_id TEXT, origin_peer TEXT, deleted_at TEXT, deleted_by_peer TEXT)";

/// Before sync: no `global_id`, `origin_peer` or `deleted_at`.
pub const PRE_SYNC: &str = "CREATE TABLE memories (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  content TEXT NOT NULL,
  memory_type TEXT NOT NULL DEFAULT 'note',
  tags TEXT,
  metadata TEXT,
  embedding BLOB,
  created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)";

/// A row of a Ruby `memories` table with the sync columns.
#[derive(Debug, Clone)]
pub struct RubyRow {
    pub content: String,
    pub memory_type: String,
    pub tags: Option<String>,
    pub created_at: Option<String>,
    pub global_id: Option<String>,
    pub deleted_at: Option<String>,
}

impl RubyRow {
    /// A valid live note created at 2026-03-01T10:00:00.000Z, without tags.
    pub fn note(global_id: &str, content: &str) -> Self {
        RubyRow {
            content: content.to_string(),
            memory_type: "note".to_string(),
            tags: Some("[]".to_string()),
            created_at: Some("2026-03-01T10:00:00.000Z".to_string()),
            global_id: Some(global_id.to_string()),
            deleted_at: None,
        }
    }
}

/// Creates the Ruby file `name` ("global.db" or "projects/<project>.db")
/// under `dir` with the `memories` table `ddl`, in WAL mode like the Ruby
/// server's files; the returned connection adds rows.
pub fn ruby_file(dir: &Path, name: &str, ddl: &str) -> Connection {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch(ddl).unwrap();
    conn
}

/// Inserts `row` into a Ruby file with the sync columns, with an
/// `origin_peer` as every real row has; returns the row's Ruby id.
pub fn insert(conn: &Connection, row: &RubyRow) -> i64 {
    conn.execute(
        "INSERT INTO memories (content, memory_type, tags, metadata, created_at, updated_at, global_id, origin_peer, deleted_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?4, ?5, 'ruby-peer-id', ?6)",
        params![
            row.content,
            row.memory_type,
            row.tags,
            row.created_at,
            row.global_id,
            row.deleted_at
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}
