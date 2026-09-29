//! Chunk embeddings and the record of which model produced them.

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, embedding_blob};
use crate::error::{Error, Result};
use crate::memory::Embedded;

/// Live memories without vectors, over the `memories` table aliased `m`.
const PENDING: &str =
    "m.deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.memory_id = m.id)";

/// Inserts the chunks of one memory and records their model unless a model is
/// already recorded (callers check compatibility before embedding).
pub(super) fn insert_chunks(conn: &Connection, memory_id: i64, embedded: &Embedded) -> Result<()> {
    let mut insert =
        conn.prepare("INSERT INTO chunks (memory_id, chunk_index, embedding) VALUES (?1, ?2, ?3)")?;
    for (index, vector) in embedded.chunks.iter().enumerate() {
        insert.execute(params![memory_id, index as i64, embedding_blob(vector)])?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('embedding_model', ?1)",
        [&embedded.model_id],
    )?;
    Ok(())
}

impl Database {
    /// The model that produced the stored vectors; `None` before the first embedding.
    pub fn stored_embedding_model(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Live memories that have no chunks yet, as `(id, content)`, by id.
    pub fn pending_embeddings(&self) -> Result<Vec<(i64, String)>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT m.id, m.content FROM memories m WHERE {PENDING} ORDER BY m.id"
        ))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn pending_embedding_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            &format!("SELECT count(*) FROM memories m WHERE {PENDING}"),
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Stores the embeddings of a live memory. Returns false and stores
    /// nothing when the memory already has vectors, as it does when another
    /// process embedded it first.
    pub fn add_embeddings(&mut self, memory_id: i64, embedded: &Embedded) -> Result<bool> {
        let tx = self.write_transaction()?;
        let state: Option<(bool, bool)> = tx
            .query_row(
                &format!("SELECT m.deleted_at IS NULL, {PENDING} FROM memories m WHERE m.id = ?1"),
                [memory_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match state {
            Some((true, true)) => {
                insert_chunks(&tx, memory_id, embedded)?;
                tx.commit()?;
                Ok(true)
            }
            Some((true, false)) => Ok(false),
            _ => Err(Error::NotFound(memory_id)),
        }
    }

    /// Deletes every stored vector and the recorded model.
    pub fn clear_embeddings(&mut self) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM chunks", [])?;
        tx.execute("DELETE FROM meta WHERE key = 'embedding_model'", [])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::db::test_support::record;
    use crate::error::Error;
    use crate::memory::Embedded;

    const T0: &str = "2026-09-01T10:00:00.000Z";

    fn embedded(model_id: &str) -> Embedded {
        Embedded {
            model_id: model_id.into(),
            chunks: vec![vec![0.0, 1.0, 0.0]],
        }
    }

    #[test]
    fn no_model_is_recorded_before_the_first_embedding() {
        let mut db = Database::open_in_memory().unwrap();
        assert_eq!(db.stored_embedding_model().unwrap(), None);
        db.insert_memory(&record("a", None, T0), Some(&embedded("m1")))
            .unwrap();
        assert_eq!(db.stored_embedding_model().unwrap().as_deref(), Some("m1"));
    }

    #[test]
    fn pending_lists_live_memories_without_chunks() {
        let mut db = Database::open_in_memory().unwrap();
        let bare = db.insert_memory(&record("bare", None, T0), None).unwrap();
        db.insert_memory(&record("embedded", None, T0), Some(&embedded("m1")))
            .unwrap();
        let deleted = db
            .insert_memory(&record("deleted", None, T0), None)
            .unwrap();
        db.delete(deleted, T0).unwrap();
        assert_eq!(
            db.pending_embeddings().unwrap(),
            [(bare, "bare".to_string())]
        );
        assert_eq!(db.pending_embedding_count().unwrap(), 1);
    }

    #[test]
    fn adding_embeddings_resolves_a_pending_memory() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db.insert_memory(&record("bare", None, T0), None).unwrap();
        assert!(db.add_embeddings(id, &embedded("m1")).unwrap());
        assert!(db.pending_embeddings().unwrap().is_empty());
        assert_eq!(db.stored_embedding_model().unwrap().as_deref(), Some("m1"));
    }

    #[test]
    fn a_memory_that_already_has_vectors_keeps_them() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db
            .insert_memory(&record("embedded", None, T0), Some(&embedded("m1")))
            .unwrap();
        let other = Embedded {
            model_id: "m1".into(),
            chunks: vec![vec![1.0, 0.0, 0.0], vec![1.0, 0.0, 0.0]],
        };
        assert!(!db.add_embeddings(id, &other).unwrap());
        let blobs: Vec<Vec<u8>> = db
            .conn
            .prepare("SELECT embedding FROM chunks WHERE memory_id = ?1")
            .unwrap()
            .query_map([id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let zero_one_zero = [0, 0, 0, 0, 0, 0, 0x80, 0x3f, 0, 0, 0, 0];
        assert_eq!(blobs, [zero_one_zero.to_vec()]);
    }

    #[test]
    fn tombstoned_or_unknown_memories_cannot_get_embeddings() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db.insert_memory(&record("x", None, T0), None).unwrap();
        db.delete(id, T0).unwrap();
        assert!(matches!(
            db.add_embeddings(id, &embedded("m1")),
            Err(Error::NotFound(missing)) if missing == id
        ));
        assert!(matches!(
            db.add_embeddings(77, &embedded("m1")),
            Err(Error::NotFound(77))
        ));
    }

    #[test]
    fn clearing_embeddings_makes_every_live_memory_pending() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(&record("a", None, T0), Some(&embedded("m1")))
            .unwrap();
        db.insert_memory(&record("b", None, T0), Some(&embedded("m1")))
            .unwrap();
        db.clear_embeddings().unwrap();
        assert_eq!(db.pending_embedding_count().unwrap(), 2);
        assert_eq!(db.stored_embedding_model().unwrap(), None);
    }
}
