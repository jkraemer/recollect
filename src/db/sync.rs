//! What sync reads from and writes to the memories table.

use rusqlite::Row;

use super::Database;
use super::memories::{
    INSERT_MEMORY, UNLESS_STORED, apply_tombstone, insert_row, json_array, memory_type_at, tags_at,
};
use crate::error::Result;
use crate::memory::{SyncRecord, Tombstone};

/// What applying a peer's changes did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SyncApplied {
    /// Live memories that were new here.
    pub memories: usize,
    /// Memories that were deleted here, or arrived deleted.
    pub deletions: usize,
}

/// The columns `sync_record_from_row` reads.
const SYNC_COLUMNS: &str = "global_id, project, memory_type, content, tags, origin_peer, created_at, deleted_at, deleted_by_peer";

fn sync_record_from_row(row: &Row<'_>) -> rusqlite::Result<SyncRecord> {
    Ok(SyncRecord {
        global_id: row.get(0)?,
        project: row.get(1)?,
        memory_type: memory_type_at(row, 2)?,
        content: row.get(3)?,
        tags: tags_at(row, 4)?,
        origin_peer: row.get(5)?,
        created_at: row.get(6)?,
        deleted_at: row.get(7)?,
        deleted_by_peer: row.get(8)?,
    })
}

impl Database {
    /// Every memory, live or deleted, as `(global_id, deleted)`, ordered by
    /// global id.
    pub fn sync_manifest(&self) -> Result<Vec<(String, bool)>> {
        let mut statement = self
            .conn
            .prepare("SELECT global_id, deleted_at IS NOT NULL FROM memories ORDER BY global_id")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The full rows of the memories with these global ids, live or deleted,
    /// ordered by global id. Unknown ids are skipped.
    pub fn sync_records(&self, global_ids: &[String]) -> Result<Vec<SyncRecord>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SYNC_COLUMNS} FROM memories
             WHERE global_id IN (SELECT value FROM json_each(?1)) ORDER BY global_id"
        ))?;
        let rows = statement.query_map([json_array(global_ids)], sync_record_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The tombstones of the deleted memories among these global ids, ordered
    /// by global id.
    pub fn sync_tombstones(&self, global_ids: &[String]) -> Result<Vec<Tombstone>> {
        let mut statement = self.conn.prepare(
            "SELECT global_id, deleted_at, deleted_by_peer FROM memories
             WHERE deleted_at IS NOT NULL AND global_id IN (SELECT value FROM json_each(?1))
             ORDER BY global_id",
        )?;
        let rows = statement.query_map([json_array(global_ids)], |row| {
            Ok(Tombstone {
                global_id: row.get(0)?,
                deleted_at: row.get(1)?,
                deleted_by_peer: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Applies what a peer sent, in one transaction. A record whose
    /// `global_id` is already stored is skipped, so nothing stored changes and
    /// no tombstone comes back to life; a deleted record is stored as a
    /// tombstone, and deletes the copy that is live here. Tombstones delete
    /// the live memories they name and leave unknown and deleted ones alone.
    /// New live memories have no vectors yet. A failure changes nothing.
    pub fn apply_sync(
        &mut self,
        records: &[SyncRecord],
        tombstones: &[Tombstone],
    ) -> Result<SyncApplied> {
        let tx = self.write_transaction()?;
        let insert = format!("{INSERT_MEMORY} {UNLESS_STORED}");
        let mut applied = SyncApplied::default();
        for record in records {
            let inserted = insert_row(&tx, &insert, &record.to_new_record())?;
            match record.tombstone() {
                None => applied.memories += inserted,
                Some(tombstone) => {
                    if apply_tombstone(&tx, &tombstone)? {
                        applied.deletions += 1;
                    }
                }
            }
        }
        for tombstone in tombstones {
            if apply_tombstone(&tx, tombstone)? {
                applied.deletions += 1;
            }
        }
        tx.commit()?;
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::test_support::{LATER, T0, fts_hits, record};
    use crate::db::{Database, SyncApplied};
    use crate::error::Error;
    use crate::memory::{MemoryType, NewRecord, SyncRecord, Tombstone};

    /// A live memory as a peer sends it; `record(content, ..)` has the same global id.
    fn live(content: &str) -> SyncRecord {
        SyncRecord {
            global_id: format!("test-{content}"),
            project: Some("p".into()),
            memory_type: MemoryType::Todo,
            content: content.into(),
            tags: vec!["tagged".into()],
            origin_peer: Some("SHA256:origin".into()),
            created_at: T0.into(),
            deleted_at: None,
            deleted_by_peer: None,
        }
    }

    /// The same memory as a peer sends it after deleting it.
    fn deleted(content: &str) -> SyncRecord {
        SyncRecord {
            content: String::new(),
            tags: Vec::new(),
            deleted_at: Some(LATER.into()),
            deleted_by_peer: Some("SHA256:deleter".into()),
            ..live(content)
        }
    }

    fn tombstone(content: &str) -> Tombstone {
        deleted(content).tombstone().unwrap()
    }

    fn ids(contents: &[&str]) -> Vec<String> {
        contents
            .iter()
            .map(|content| format!("test-{content}"))
            .collect()
    }

    #[test]
    fn the_manifest_lists_every_memory_with_its_state_by_global_id() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(&record("b", None, T0), None).unwrap();
        let gone = db.insert_memory(&record("a", None, T0), None).unwrap();
        db.delete(gone, LATER).unwrap();
        assert_eq!(
            db.sync_manifest().unwrap(),
            [("test-a".to_string(), true), ("test-b".to_string(), false)]
        );
    }

    #[test]
    fn records_come_back_in_full_whether_live_or_deleted() {
        let mut db = Database::open_in_memory().unwrap();
        let kept = NewRecord {
            memory_type: MemoryType::Todo,
            tags: vec!["tagged".into()],
            origin_peer: Some("SHA256:origin".into()),
            ..record("kept", Some("p"), T0)
        };
        db.insert_memory(&kept, None).unwrap();
        let gone = db
            .insert_memory(&record("gone", Some("p"), T0), None)
            .unwrap();
        db.delete(gone, LATER).unwrap();
        db.insert_memory(&record("unasked", None, T0), None)
            .unwrap();

        let records = db.sync_records(&ids(&["kept", "gone", "unknown"])).unwrap();

        assert_eq!(
            records,
            [
                SyncRecord {
                    global_id: "test-gone".into(),
                    project: Some("p".into()),
                    memory_type: MemoryType::Note,
                    content: String::new(),
                    tags: Vec::new(),
                    origin_peer: None,
                    created_at: T0.into(),
                    deleted_at: Some(LATER.into()),
                    deleted_by_peer: None,
                },
                live("kept"),
            ]
        );
    }

    #[test]
    fn records_are_selected_from_more_ids_than_sqlite_binds_parameters() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(&record("wanted", None, T0), None).unwrap();
        let mut many: Vec<String> = (0..40_000).map(|n| format!("absent-{n}")).collect();
        many.push("test-wanted".into());
        assert_eq!(db.sync_records(&many).unwrap().len(), 1);
    }

    #[test]
    fn tombstones_are_returned_only_for_deleted_memories() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(&record("live", None, T0), None).unwrap();
        db.apply_sync(&[deleted("gone")], &[]).unwrap();
        assert_eq!(
            db.sync_tombstones(&ids(&["live", "gone", "unknown"]))
                .unwrap(),
            [tombstone("gone")]
        );
    }

    #[test]
    fn new_live_records_are_inserted_pending_and_indexed() {
        let mut db = Database::open_in_memory().unwrap();
        let applied = db.apply_sync(&[live("alpha"), live("beta")], &[]).unwrap();
        assert_eq!(
            applied,
            SyncApplied {
                memories: 2,
                deletions: 0
            }
        );
        assert_eq!(db.sync_records(&ids(&["alpha"])).unwrap(), [live("alpha")]);
        assert_eq!(fts_hits(&db, "alpha"), 1);
        assert_eq!(fts_hits(&db, "tagged"), 2);
        assert_eq!(db.pending_embedding_count().unwrap(), 2);
    }

    #[test]
    fn a_deleted_record_arrives_as_a_tombstone_that_cannot_be_found() {
        let mut db = Database::open_in_memory().unwrap();
        let applied = db.apply_sync(&[deleted("gone")], &[]).unwrap();
        assert_eq!(
            applied,
            SyncApplied {
                memories: 0,
                deletions: 1
            }
        );
        assert_eq!(
            db.sync_manifest().unwrap(),
            [("test-gone".to_string(), true)]
        );
        assert_eq!(db.sync_records(&ids(&["gone"])).unwrap(), [deleted("gone")]);
        assert_eq!(db.live_count().unwrap(), 0);
        assert_eq!(db.pending_embedding_count().unwrap(), 0);
    }

    #[test]
    fn a_deleted_record_that_still_carries_text_is_stored_blank_and_unindexed() {
        let mut db = Database::open_in_memory().unwrap();
        let with_text = SyncRecord {
            deleted_at: Some(LATER.into()),
            deleted_by_peer: Some("SHA256:deleter".into()),
            ..live("gone")
        };
        let applied = db.apply_sync(&[with_text], &[]).unwrap();
        assert_eq!(
            applied,
            SyncApplied {
                memories: 0,
                deletions: 1
            }
        );
        assert_eq!(fts_hits(&db, "gone"), 0);
        assert_eq!(fts_hits(&db, "tagged"), 0);
        assert_eq!(db.sync_records(&ids(&["gone"])).unwrap(), [deleted("gone")]);
    }

    #[test]
    fn tombstones_delete_live_memories_keeping_when_and_who() {
        let mut db = Database::open_in_memory().unwrap();
        db.apply_sync(&[live("doomed"), live("kept")], &[]).unwrap();
        let applied = db.apply_sync(&[], &[tombstone("doomed")]).unwrap();
        assert_eq!(
            applied,
            SyncApplied {
                memories: 0,
                deletions: 1
            }
        );
        assert_eq!(
            db.sync_tombstones(&ids(&["doomed"])).unwrap(),
            [tombstone("doomed")]
        );
        assert_eq!(fts_hits(&db, "doomed"), 0);
        assert_eq!(fts_hits(&db, "kept"), 1);
    }

    #[test]
    fn applying_the_same_changes_twice_changes_nothing() {
        let mut db = Database::open_in_memory().unwrap();
        let records = [live("alpha"), deleted("gone")];
        let tombstones = [tombstone("alpha")];
        db.apply_sync(&records, &tombstones).unwrap();
        let before = db.sync_records(&ids(&["alpha", "gone"])).unwrap();
        assert_eq!(
            db.apply_sync(&records, &tombstones).unwrap(),
            SyncApplied::default()
        );
        assert_eq!(db.sync_records(&ids(&["alpha", "gone"])).unwrap(), before);
    }

    #[test]
    fn a_memory_deleted_here_keeps_its_own_tombstone_and_never_comes_back() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db.insert_memory(&record("x", Some("p"), T0), None).unwrap();
        db.delete(id, T0).unwrap();
        let applied = db.apply_sync(&[live("x")], &[tombstone("x")]).unwrap();
        assert_eq!(applied, SyncApplied::default());
        let stored = db.sync_tombstones(&ids(&["x"])).unwrap();
        assert_eq!(stored[0].deleted_at, T0, "the local deletion time stays");
        assert_eq!(stored[0].deleted_by_peer, None);
        assert_eq!(db.live_count().unwrap(), 0);
    }

    #[test]
    fn a_deleted_record_tombstones_the_copy_that_is_live_here() {
        let mut db = Database::open_in_memory().unwrap();
        db.apply_sync(&[live("x")], &[]).unwrap();
        let applied = db.apply_sync(&[deleted("x")], &[]).unwrap();
        assert_eq!(
            applied,
            SyncApplied {
                memories: 0,
                deletions: 1
            }
        );
        assert_eq!(db.live_count().unwrap(), 0);
        assert_eq!(fts_hits(&db, "x"), 0);
    }

    #[test]
    fn a_failed_apply_changes_nothing() {
        let mut db = Database::open_in_memory().unwrap();
        db.apply_sync(&[live("first")], &[]).unwrap();
        db.conn
            .execute_batch(
                "CREATE TEMP TRIGGER refuse_third BEFORE INSERT ON memories
                 WHEN NEW.content = 'third' BEGIN SELECT RAISE(ABORT, 'refused'); END",
            )
            .unwrap();
        let err = db
            .apply_sync(&[live("second"), live("third")], &[tombstone("first")])
            .unwrap_err();
        assert!(matches!(&err, Error::Database(_)), "{err}");
        assert_eq!(
            db.sync_manifest().unwrap(),
            [("test-first".to_string(), false)]
        );
        assert_eq!(fts_hits(&db, "second"), 0);
    }
}
