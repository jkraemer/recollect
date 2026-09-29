//! Chunk embeddings and the record of which model produced them.

use rusqlite::{Connection, params};

use super::embedding_blob;
use crate::error::Result;
use crate::memory::Embedded;

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
