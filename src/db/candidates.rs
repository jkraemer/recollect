//! Candidate ids for hybrid search, one query per arm.

use rusqlite::params_from_iter;
use rusqlite::types::Value;

use super::{Database, embedding_blob};
use crate::error::Result;
use crate::filter::Filter;

impl Database {
    /// Live memories matching the FTS5 query under `filter`, best BM25 first.
    pub fn fts_candidates(
        &self,
        fts_query: &str,
        filter: &Filter,
        limit: usize,
    ) -> Result<Vec<i64>> {
        let (conditions, filter_params) = filter.to_sql();
        let sql = format!(
            "SELECT m.id FROM memories_fts JOIN memories m ON m.id = memories_fts.rowid
             WHERE memories_fts MATCH ? AND {conditions}
             ORDER BY bm25(memories_fts), m.id DESC LIMIT ?"
        );
        let mut params = vec![Value::Text(fts_query.to_string())];
        params.extend(filter_params);
        params.push(Value::Integer(limit as i64));
        self.ids(&sql, params)
    }

    /// Live memories under `filter` whose closest chunk lies within
    /// `max_distance` (cosine) of `query`, one entry per memory, closest first.
    pub fn vector_candidates(
        &self,
        query: &[f32],
        filter: &Filter,
        max_distance: f64,
        limit: usize,
    ) -> Result<Vec<i64>> {
        let (conditions, filter_params) = filter.to_sql();
        let sql = format!(
            "SELECT c.memory_id, MIN(vec_distance_cosine(c.embedding, ?)) AS distance
             FROM chunks c JOIN memories m ON m.id = c.memory_id
             WHERE {conditions}
             GROUP BY c.memory_id HAVING distance <= ?
             ORDER BY distance, c.memory_id DESC LIMIT ?"
        );
        let mut params = vec![Value::Blob(embedding_blob(query))];
        params.extend(filter_params);
        params.push(Value::Real(max_distance));
        params.push(Value::Integer(limit as i64));
        self.ids(&sql, params)
    }

    fn ids(&self, sql: &str, params: Vec<Value>) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map(params_from_iter(params), |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::db::test_support::record;
    use crate::filter::Filter;
    use crate::memory::{Embedded, ProjectRef};
    use crate::search::fts_query::build_fts_query;

    const T0: &str = "2026-09-01T10:00:00.000Z";

    fn fts(db: &Database, text: &str, filter: &Filter) -> Vec<i64> {
        db.fts_candidates(&build_fts_query(text).unwrap(), filter, 10)
            .unwrap()
    }

    fn vectors(chunks: &[[f32; 3]]) -> Embedded {
        Embedded {
            model_id: "test".into(),
            chunks: chunks.iter().map(|c| c.to_vec()).collect(),
        }
    }

    #[test]
    fn memories_matching_more_terms_rank_first() {
        let mut db = Database::open_in_memory().unwrap();
        let one = db
            .insert_memory(&record("sqlite wal", None, T0), None)
            .unwrap();
        let both = db
            .insert_memory(&record("sqlite wal busy timeout", None, T0), None)
            .unwrap();
        db.insert_memory(&record("unrelated", None, T0), None)
            .unwrap();
        assert_eq!(fts(&db, "busy sqlite", &Filter::default()), [both, one]);
    }

    #[test]
    fn full_text_candidates_respect_the_filter_and_skip_tombstones() {
        let mut db = Database::open_in_memory().unwrap();
        let kept = db
            .insert_memory(&record("deploy notes", Some("a"), T0), None)
            .unwrap();
        db.insert_memory(&record("deploy notes for b", Some("b"), T0), None)
            .unwrap();
        let gone = db
            .insert_memory(&record("deploy notes gone", Some("a"), T0), None)
            .unwrap();
        db.delete(gone, T0).unwrap();
        let only_a = Filter {
            project: Some(ProjectRef::Named("a".into())),
            ..Filter::default()
        };
        assert_eq!(fts(&db, "deploy", &only_a), [kept]);
    }

    #[test]
    fn hostile_query_text_runs_as_plain_words() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db
            .insert_memory(&record("auth bug in login", None, T0), None)
            .unwrap();
        for text in [
            r#"auth-bug: "unclosed"#,
            "NEAR(auth login)",
            "auth AND OR NOT",
            "login*",
            "content:login auth",
            "auth\0login",
        ] {
            let hits = fts(&db, text, &Filter::default());
            assert_eq!(hits, [id], "{text}");
        }
    }

    #[test]
    fn full_text_matching_folds_case_and_diacritics() {
        let mut db = Database::open_in_memory().unwrap();
        let id = db
            .insert_memory(&record("Übergröße im Café", None, T0), None)
            .unwrap();
        for text in ["übergröße", "cafe", "CAFÉ"] {
            assert_eq!(fts(&db, text, &Filter::default()), [id], "{text}");
        }
    }

    #[test]
    fn vector_candidates_are_closest_first_one_per_memory_within_the_distance() {
        let mut db = Database::open_in_memory().unwrap();
        let exact = db
            .insert_memory(
                &record("exact", None, T0),
                Some(&vectors(&[[1.0, 0.0, 0.0]])),
            )
            .unwrap();
        let near = db
            .insert_memory(
                &record("near", None, T0),
                Some(&vectors(&[[0.0, 0.0, 1.0], [0.8, 0.6, 0.0]])),
            )
            .unwrap();
        db.insert_memory(
            &record("orthogonal", None, T0),
            Some(&vectors(&[[0.0, 1.0, 0.0]])),
        )
        .unwrap();
        db.insert_memory(
            &record("opposite", None, T0),
            Some(&vectors(&[[-1.0, 0.0, 0.0]])),
        )
        .unwrap();

        let query = [1.0, 0.0, 0.0];
        assert_eq!(
            db.vector_candidates(&query, &Filter::default(), 0.5, 10)
                .unwrap(),
            [exact, near]
        );
        assert_eq!(
            db.vector_candidates(&query, &Filter::default(), 2.0, 10)
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            db.vector_candidates(&query, &Filter::default(), 2.0, 1)
                .unwrap(),
            [exact]
        );
    }

    #[test]
    fn vector_candidates_respect_the_filter() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(
            &record("global", None, T0),
            Some(&vectors(&[[1.0, 0.0, 0.0]])),
        )
        .unwrap();
        let named = db
            .insert_memory(
                &record("named", Some("p"), T0),
                Some(&vectors(&[[0.9, 0.1, 0.0]])),
            )
            .unwrap();
        let only_p = Filter {
            project: Some(ProjectRef::Named("p".into())),
            ..Filter::default()
        };
        assert_eq!(
            db.vector_candidates(&[1.0, 0.0, 0.0], &only_p, 2.0, 10)
                .unwrap(),
            [named]
        );
    }
}
