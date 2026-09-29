//! Read queries over live memories.

use std::collections::HashMap;

use rusqlite::params_from_iter;

use super::memories::{MEMORY_COLUMNS, memory_from_row};
use super::{Database, sql_limit};
use crate::error::Result;
use crate::filter::Filter;
use crate::memory::Memory;

impl Database {
    /// Live memories matching `filter`, newest first.
    pub fn list(&self, filter: &Filter, limit: usize) -> Result<Vec<Memory>> {
        let (conditions, mut params) = filter.to_sql();
        params.push(sql_limit(limit));
        let sql = format!(
            "SELECT {MEMORY_COLUMNS} FROM memories m WHERE {conditions}
             ORDER BY m.created_at DESC, m.id DESC LIMIT ?"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params), memory_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The live memories among `ids`, keyed by id.
    pub fn get_many(&self, ids: &[i64]) -> Result<HashMap<i64, Memory>> {
        // One JSON array parameter: SQLite caps the number of bound parameters.
        let sql = format!(
            "SELECT {MEMORY_COLUMNS} FROM memories m
             WHERE m.deleted_at IS NULL AND m.id IN (SELECT value FROM json_each(?1))"
        );
        let ids = serde_json::to_string(ids).expect("a list of integers always serializes");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map([ids], memory_from_row)?;
        rows.map(|row| Ok(row.map(|memory| (memory.id, memory))?))
            .collect()
    }

    /// How many memories are not tombstoned.
    pub fn live_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM memories WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Live-memory counts per project, the global bucket named `global`, by name.
    pub fn project_counts(&self) -> Result<Vec<(String, usize)>> {
        let mut statement = self.conn.prepare(
            "SELECT COALESCE(project, 'global') AS name, count(*) FROM memories
             WHERE deleted_at IS NULL GROUP BY project ORDER BY name",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Tag frequencies over live memories matching `filter`: most frequent first, ties by tag.
    pub fn tag_counts(&self, filter: &Filter, top: usize) -> Result<Vec<(String, usize)>> {
        let (conditions, mut params) = filter.to_sql();
        params.push(sql_limit(top));
        let sql = format!(
            "SELECT t.value, count(*) AS uses FROM memories m, json_each(m.tags) AS t
             WHERE {conditions} GROUP BY t.value ORDER BY uses DESC, t.value LIMIT ?"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::db::test_support::record;
    use crate::filter::Filter;
    use crate::memory::{MemoryType, NewRecord, ProjectRef};

    fn day(n: u32) -> String {
        format!("2026-09-{n:02}T10:00:00.000Z")
    }

    /// Memories 1..=4: note/recollect/day1 [a], todo/recollect/day2 [a,b],
    /// session/global/day3 [b], note/other/day4 [].
    fn seeded() -> Database {
        let mut db = Database::open_in_memory().unwrap();
        let rows = [
            NewRecord {
                tags: vec!["a".into()],
                ..record("one", Some("recollect"), &day(1))
            },
            NewRecord {
                memory_type: MemoryType::Todo,
                tags: vec!["a".into(), "b".into()],
                ..record("two", Some("recollect"), &day(2))
            },
            NewRecord {
                memory_type: MemoryType::Session,
                tags: vec!["b".into()],
                ..record("three", None, &day(3))
            },
            record("four", Some("other"), &day(4)),
        ];
        for row in &rows {
            db.insert_memory(row, None).unwrap();
        }
        db
    }

    fn contents(db: &Database, filter: &Filter) -> Vec<String> {
        db.list(filter, 20)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect()
    }

    #[test]
    fn list_returns_newest_first_within_the_limit() {
        let db = seeded();
        assert_eq!(
            contents(&db, &Filter::default()),
            ["four", "three", "two", "one"]
        );
        let limited: Vec<String> = db
            .list(&Filter::default(), 2)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect();
        assert_eq!(limited, ["four", "three"]);
    }

    #[test]
    fn list_puts_the_newer_id_first_when_timestamps_tie() {
        let mut db = Database::open_in_memory().unwrap();
        let first = db
            .insert_memory(&record("first", None, &day(1)), None)
            .unwrap();
        let second = db
            .insert_memory(&record("second", None, &day(1)), None)
            .unwrap();
        let ids: Vec<i64> = db
            .list(&Filter::default(), 10)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, [second, first]);
    }

    #[test]
    fn list_skips_tombstones() {
        let mut db = seeded();
        db.delete(4, &day(5)).unwrap();
        assert_eq!(contents(&db, &Filter::default()), ["three", "two", "one"]);
    }

    #[test]
    fn list_filters_by_project_type_tags_and_dates() {
        let db = seeded();
        let named = Filter {
            project: Some(ProjectRef::Named("recollect".into())),
            ..Filter::default()
        };
        assert_eq!(contents(&db, &named), ["two", "one"]);
        let global = Filter {
            project: Some(ProjectRef::Global),
            ..Filter::default()
        };
        assert_eq!(contents(&db, &global), ["three"]);
        let types = Filter {
            types: vec![MemoryType::Todo, MemoryType::Session],
            ..Filter::default()
        };
        assert_eq!(contents(&db, &types), ["three", "two"]);
        let all_tags = Filter {
            tags: vec!["a".into(), "b".into()],
            ..Filter::default()
        };
        assert_eq!(contents(&db, &all_tags), ["two"]);
        let dates = Filter {
            since: Some(day(2)),
            until: Some(day(3)),
            ..Filter::default()
        };
        assert_eq!(
            contents(&db, &dates),
            ["three", "two"],
            "both bounds are inclusive"
        );
    }

    #[test]
    fn get_many_returns_live_memories_by_id() {
        let mut db = seeded();
        db.delete(2, &day(5)).unwrap();
        let found = db.get_many(&[1, 2, 3, 99]).unwrap();
        let mut ids: Vec<i64> = found.keys().copied().collect();
        ids.sort();
        assert_eq!(ids, [1, 3]);
        assert_eq!(found[&3].content, "three");
        assert!(db.get_many(&[]).unwrap().is_empty());
    }

    #[test]
    fn get_many_takes_more_ids_than_sqlite_binds_parameters() {
        let db = seeded();
        let ids: Vec<i64> = (1..=40_000).collect();
        assert_eq!(db.get_many(&ids).unwrap().len(), 4);
    }

    #[test]
    fn counts_cover_live_memories_and_name_the_global_bucket() {
        let mut db = seeded();
        db.delete(4, &day(5)).unwrap();
        assert_eq!(db.live_count().unwrap(), 3);
        assert_eq!(
            db.project_counts().unwrap(),
            [("global".to_string(), 1), ("recollect".to_string(), 2)]
        );
    }

    #[test]
    fn tag_counts_rank_by_frequency_then_name_under_the_filter() {
        let db = seeded();
        assert_eq!(
            db.tag_counts(&Filter::default(), 20).unwrap(),
            [("a".to_string(), 2), ("b".to_string(), 2)]
        );
        assert_eq!(
            db.tag_counts(&Filter::default(), 1).unwrap(),
            [("a".to_string(), 2)]
        );
        let session = Filter {
            types: vec![MemoryType::Session],
            ..Filter::default()
        };
        assert_eq!(db.tag_counts(&session, 20).unwrap(), [("b".to_string(), 1)]);
    }
}
