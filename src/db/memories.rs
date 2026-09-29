//! Memory rows: insert, read, tombstone.

use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};

use super::{Database, chunks};
use crate::error::{Error, Result};
use crate::memory::{Embedded, Memory, MemoryType, NewRecord};

/// The columns `memory_from_row` reads, from the `memories` table aliased `m`.
pub(super) const MEMORY_COLUMNS: &str =
    "m.id, m.global_id, m.project, m.memory_type, m.content, m.tags, m.created_at";

pub(super) fn memory_from_row(row: &Row<'_>) -> rusqlite::Result<Memory> {
    let memory_type: String = row.get(3)?;
    let tags: String = row.get(5)?;
    Ok(Memory {
        id: row.get(0)?,
        global_id: row.get(1)?,
        project: row.get(2)?,
        memory_type: MemoryType::from_db(&memory_type).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                Type::Text,
                format!("unknown memory type {memory_type:?}").into(),
            )
        })?,
        content: row.get(4)?,
        tags: serde_json::from_str(&tags).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
        })?,
        created_at: row.get(6)?,
    })
}

fn tags_json(tags: &[String]) -> String {
    serde_json::to_string(tags).expect("a list of strings always serializes")
}

impl Database {
    /// Inserts a live memory and, when given, its chunk embeddings in one transaction.
    pub fn insert_memory(
        &mut self,
        record: &NewRecord,
        embedded: Option<&Embedded>,
    ) -> Result<i64> {
        let tx = self.write_transaction()?;
        tx.execute(
            "INSERT INTO memories (global_id, project, memory_type, content, tags, origin_peer, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.global_id,
                record.project,
                record.memory_type.as_str(),
                record.content,
                tags_json(&record.tags),
                record.origin_peer,
                record.created_at,
            ],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(embedded) = embedded {
            chunks::insert_chunks(&tx, id, embedded)?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// A live memory; unknown and tombstoned ids are `NotFound`.
    pub fn get(&self, id: i64) -> Result<Memory> {
        self.conn
            .query_row(
                &format!("SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.id = ?1 AND m.deleted_at IS NULL"),
                [id],
                memory_from_row,
            )
            .optional()?
            .ok_or(Error::NotFound(id))
    }

    /// Tombstones a live memory: drops its chunks and blanks its text. The row
    /// stays so sync can propagate the delete.
    pub fn delete(&mut self, id: i64, deleted_at: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM chunks WHERE memory_id = ?1", [id])?;
        let changed = tx.execute(
            "UPDATE memories SET deleted_at = ?2, deleted_by_peer = NULL, content = '', tags = '[]'
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id, deleted_at],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(id));
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::db::Database;
    use crate::db::test_support::{concurrently, record};
    use crate::error::Error;
    use crate::memory::{Embedded, MemoryType, NewRecord};

    const T0: &str = "2026-09-01T10:00:00.000Z";
    const LATER: &str = "2026-09-02T10:00:00.000Z";

    fn fts_hits(db: &Database, query: &str) -> i64 {
        db.conn
            .query_row(
                "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH ?1",
                [query],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn embedded(model_id: &str, chunks: usize) -> Embedded {
        Embedded {
            model_id: model_id.into(),
            chunks: vec![vec![1.0, 0.0, 0.0]; chunks],
        }
    }

    #[test]
    fn inserted_memories_read_back_unchanged() {
        let mut db = Database::open_in_memory().unwrap();
        let new = NewRecord {
            memory_type: MemoryType::Todo,
            tags: vec!["decision".into(), "sync".into()],
            ..record("first memory", Some("recollect"), T0)
        };
        let id = db.insert_memory(&new, None).unwrap();
        let memory = db.get(id).unwrap();
        assert_eq!(memory.id, id);
        assert_eq!(memory.global_id, "test-first memory");
        assert_eq!(memory.project.as_deref(), Some("recollect"));
        assert_eq!(memory.memory_type, MemoryType::Todo);
        assert_eq!(memory.content, "first memory");
        assert_eq!(memory.tags, vec!["decision", "sync"]);
        assert_eq!(memory.created_at, T0);
        let global = db
            .insert_memory(&record("global memory", None, T0), None)
            .unwrap();
        assert_eq!(db.get(global).unwrap().project, None);
    }

    #[test]
    fn inserts_from_separate_connections_wait_for_each_other_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.db");
        drop(Database::open(&path).unwrap());
        let inserted = concurrently(4, |n| -> Result<(), Error> {
            let mut db = Database::open(&path)?;
            for i in 0..5 {
                db.insert_memory(&record(&format!("memory {n}-{i}"), None, T0), None)?;
            }
            Ok(())
        });
        for result in inserted {
            if let Err(err) = result {
                panic!("a concurrent insert failed: {err}");
            }
        }
    }

    #[test]
    fn unknown_ids_are_not_found() {
        let db = Database::open_in_memory().unwrap();
        assert!(matches!(db.get(42), Err(Error::NotFound(42))));
    }

    #[test]
    fn content_and_tags_are_indexed_for_full_text_search() {
        let mut db = Database::open_in_memory().unwrap();
        let new = NewRecord {
            tags: vec!["architecture".into()],
            ..record("wal mode everywhere", None, T0)
        };
        db.insert_memory(&new, None).unwrap();
        assert_eq!(fts_hits(&db, "wal"), 1);
        assert_eq!(fts_hits(&db, "architecture"), 1);
    }

    #[test]
    fn chunks_are_stored_in_order_as_little_endian_floats() {
        let mut db = Database::open_in_memory().unwrap();
        let chunks = Embedded {
            model_id: "m".into(),
            chunks: vec![vec![1.0, -2.0], vec![0.5, 0.0]],
        };
        let id = db
            .insert_memory(&record("two chunks", None, T0), Some(&chunks))
            .unwrap();
        let stored: Vec<(i64, Vec<u8>)> = db
            .conn
            .prepare("SELECT chunk_index, embedding FROM chunks WHERE memory_id = ?1 ORDER BY id")
            .unwrap()
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            stored,
            [
                (0, vec![0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0xc0]),
                (1, vec![0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00]),
            ]
        );
    }

    #[test]
    fn a_failed_chunk_insert_leaves_no_memory_behind() {
        let mut db = Database::open_in_memory().unwrap();
        db.conn
            .execute_batch(
                "CREATE TEMP TRIGGER refuse_chunks BEFORE INSERT ON chunks
                 BEGIN SELECT RAISE(ABORT, 'refused'); END",
            )
            .unwrap();
        let err = db
            .insert_memory(&record("doomed", None, T0), Some(&embedded("m", 1)))
            .unwrap_err();
        assert!(err.to_string().contains("refused"), "{err}");
        let memories: i64 = db
            .conn
            .query_row("SELECT count(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(memories, 0);
        assert_eq!(fts_hits(&db, "doomed"), 0);
    }

    #[test]
    fn embeddings_are_stored_with_the_memory_and_the_first_model_is_recorded() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db
            .insert_memory(&record("a", None, T0), Some(&embedded("model-a", 2)))
            .unwrap();
        db.insert_memory(&record("b", None, T0), Some(&embedded("model-b", 1)))
            .unwrap();
        let chunks: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM chunks WHERE memory_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(chunks, 2);
        let model: String = db
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(model, "model-a");
    }

    #[test]
    fn delete_tombstones_the_row_and_removes_text_index_and_chunks() {
        let mut db = Database::open_in_memory().unwrap();
        let doomed = NewRecord {
            tags: vec!["secret".into()],
            ..record("zanzibar plans", None, T0)
        };
        let id = db.insert_memory(&doomed, Some(&embedded("m", 2))).unwrap();
        let kept = db
            .insert_memory(&record("zanzibar trip", None, T0), Some(&embedded("m", 1)))
            .unwrap();

        db.delete(id, LATER).unwrap();

        assert!(matches!(db.get(id), Err(Error::NotFound(_))));
        let (content, tags, deleted_at): (String, String, String) = db
            .conn
            .query_row(
                "SELECT content, tags, deleted_at FROM memories WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (content.as_str(), tags.as_str(), deleted_at.as_str()),
            ("", "[]", LATER)
        );
        assert_eq!(
            fts_hits(&db, "plans"),
            0,
            "the tombstoned text must leave the index"
        );
        assert_eq!(
            fts_hits(&db, "secret"),
            0,
            "the tombstoned tags must leave the index"
        );
        assert_eq!(fts_hits(&db, "zanzibar"), 1, "other memories stay indexed");
        let chunks: Vec<i64> = db
            .conn
            .prepare("SELECT memory_id FROM chunks")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(chunks, [kept], "only the deleted memory's chunks go");
    }

    #[test]
    fn deleting_twice_or_an_unknown_id_is_not_found() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db.insert_memory(&record("x", None, T0), None).unwrap();
        db.delete(id, LATER).unwrap();
        assert!(matches!(db.delete(id, LATER), Err(Error::NotFound(_))));
        assert!(matches!(db.delete(999, LATER), Err(Error::NotFound(999))));
    }

    #[test]
    fn tombstones_cannot_be_resurrected() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db.insert_memory(&record("x", None, T0), None).unwrap();
        db.delete(id, LATER).unwrap();
        let err = db
            .conn
            .execute(
                "UPDATE memories SET deleted_at = NULL WHERE id = ?1",
                params![id],
            )
            .unwrap_err();
        assert!(err.to_string().contains("cannot resurrect"), "{err}");
    }

    #[test]
    fn the_schema_rejects_unknown_memory_types() {
        let db = Database::open_in_memory().unwrap();
        let err = db
            .conn
            .execute(
                "INSERT INTO memories (global_id, memory_type, content, created_at) VALUES ('g', '_chunk', 'c', ?1)",
                [T0],
            )
            .unwrap_err();
        assert!(err.to_string().contains("CHECK constraint failed"), "{err}");
    }
}
