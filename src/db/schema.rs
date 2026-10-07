//! Numbered schema migrations, tracked in `PRAGMA user_version`.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Error, Result};

/// `MIGRATIONS[n]` migrates the schema from version `n` to `n + 1`.
const MIGRATIONS: &[&str] = &[
    // 1: memories, their full-text index, chunk embeddings, meta.
    r#"
    CREATE TABLE memories (
      id              INTEGER PRIMARY KEY,
      global_id       TEXT NOT NULL UNIQUE,
      project         TEXT,
      memory_type     TEXT NOT NULL CHECK (memory_type IN ('note', 'todo', 'session')),
      content         TEXT NOT NULL,
      tags            TEXT NOT NULL DEFAULT '[]',
      origin_peer     TEXT,
      created_at      TEXT NOT NULL,
      deleted_at      TEXT,
      deleted_by_peer TEXT
    );
    CREATE INDEX idx_memories_project_created ON memories(project, created_at DESC);
    CREATE INDEX idx_memories_created ON memories(created_at DESC);

    CREATE VIRTUAL TABLE memories_fts USING fts5(
      content, tags, content = 'memories', content_rowid = 'id'
    );

    CREATE TRIGGER memories_fts_insert AFTER INSERT ON memories BEGIN
      INSERT INTO memories_fts(rowid, content, tags) VALUES (new.id, new.content, new.tags);
    END;

    -- An external-content FTS5 table removes an entry by replaying the values
    -- that were indexed, so this must pass OLD.content and OLD.tags: the
    -- tombstone update blanks both in the same statement.
    CREATE TRIGGER memories_fts_tombstone AFTER UPDATE OF deleted_at ON memories
    WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL BEGIN
      INSERT INTO memories_fts(memories_fts, rowid, content, tags)
      VALUES ('delete', OLD.id, OLD.content, OLD.tags);
    END;

    CREATE TRIGGER memories_no_resurrect BEFORE UPDATE OF deleted_at ON memories
    WHEN OLD.deleted_at IS NOT NULL AND NEW.deleted_at IS NULL BEGIN
      SELECT RAISE(ABORT, 'cannot resurrect a tombstoned memory');
    END;

    CREATE TABLE chunks (
      id          INTEGER PRIMARY KEY,
      memory_id   INTEGER NOT NULL REFERENCES memories(id),
      chunk_index INTEGER NOT NULL,
      embedding   BLOB NOT NULL,
      UNIQUE (memory_id, chunk_index)
    );

    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    "#,
    // 2: the machines this one syncs with, and the invites handed out to pair with them.
    r#"
    CREATE TABLE peers (
      name         TEXT PRIMARY KEY,
      fingerprint  TEXT NOT NULL UNIQUE,
      address      TEXT NOT NULL,
      added_at     TEXT NOT NULL,
      last_sync_at TEXT,
      last_error   TEXT
    );

    CREATE TABLE pairing_invites (
      secret_hash TEXT PRIMARY KEY,
      expires_at  TEXT NOT NULL,
      used_at     TEXT
    );
    "#,
];

pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Applies pending migrations in one IMMEDIATE transaction, so a second
/// process opening the database at the same time waits and then finds the
/// work done. Refuses databases written by a newer binary.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    if user_version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = user_version(&tx)?;
    if current > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
    }
    tx.commit()?;
    Ok(())
}

fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn a_version_1_database_keeps_its_memories_and_gains_the_sync_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO memories (global_id, memory_type, content, created_at)
             VALUES ('g', 'note', 'kept', '2026-09-01T10:00:00.000Z')",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), 2);
        let content: String = conn
            .query_row(
                "SELECT content FROM memories WHERE global_id = 'g'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(content, "kept");
        for table in ["peers", "pairing_invites"] {
            let rows: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
    }
}
