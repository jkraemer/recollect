//! Hybrid search: full-text and vector candidates, fused and ranked.

pub mod fts_query;
pub mod rank;

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::config::RecencyConfig;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::memory::ScoredMemory;
use crate::time::age_days;

pub struct SearchRequest<'a> {
    pub query: &'a str,
    /// The embedded query; `None` searches full text only.
    pub query_vector: Option<&'a [f32]>,
    pub filter: &'a Filter,
    pub limit: usize,
    pub max_vector_distance: f64,
    pub recency: RecencyConfig,
    pub now: DateTime<Utc>,
}

/// Each arm fetches `limit × 3` candidates; they are fused with RRF, adjusted
/// for recency, and cut to `limit`.
pub fn search(db: &Database, request: &SearchRequest<'_>) -> Result<Vec<ScoredMemory>> {
    let fts_query = fts_query::build_fts_query(request.query).ok_or(Error::EmptyQuery)?;
    let candidates = request.limit.saturating_mul(3);
    let fts = db.fts_candidates(&fts_query, request.filter, candidates)?;
    let vector = match request.query_vector {
        Some(query) => db.vector_candidates(
            query,
            request.filter,
            request.max_vector_distance,
            candidates,
        )?,
        None => Vec::new(),
    };
    let merged = rank::rrf_merge(&fts, &vector);
    let ids: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
    let memories = db.get_many(&ids)?;
    let ages: HashMap<i64, f64> = memories
        .iter()
        .map(|(id, memory)| (*id, age_days(&memory.created_at, request.now)))
        .collect();
    Ok(rank::apply_recency(merged, &ages, request.recency)
        .into_iter()
        // A memory deleted by another process since its id was fetched is skipped.
        .filter_map(|(id, score)| {
            memories.get(&id).map(|memory| ScoredMemory {
                memory: memory.clone(),
                score,
            })
        })
        .take(request.limit)
        .collect())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::db::test_support::record;
    use crate::memory::{Embedded, NewRecord};

    fn day(n: u32) -> String {
        format!("2026-09-{n:02}T10:00:00.000Z")
    }

    fn vectors(chunk: [f32; 3]) -> Embedded {
        Embedded {
            model_id: "test".into(),
            chunks: vec![chunk.to_vec()],
        }
    }

    fn request<'a>(
        query: &'a str,
        query_vector: Option<&'a [f32]>,
        filter: &'a Filter,
    ) -> SearchRequest<'a> {
        SearchRequest {
            query,
            query_vector,
            filter,
            limit: 10,
            max_vector_distance: 1.0,
            recency: RecencyConfig::default(),
            now: Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap(),
        }
    }

    fn contents(results: &[ScoredMemory]) -> Vec<&str> {
        results.iter().map(|r| r.memory.content.as_str()).collect()
    }

    #[test]
    fn without_a_query_vector_only_full_text_counts() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(
            &record("sqlite wal mode", None, &day(1)),
            Some(&vectors([1.0, 0.0, 0.0])),
        )
        .unwrap();
        db.insert_memory(&record("banana bread", None, &day(1)), None)
            .unwrap();
        let filter = Filter::default();
        let results = search(&db, &request("wal", None, &filter)).unwrap();
        assert_eq!(contents(&results), ["sqlite wal mode"]);
    }

    #[test]
    fn the_vector_arm_finds_memories_without_shared_words() {
        let mut db = Database::open_in_memory().unwrap();
        db.insert_memory(
            &record("write-ahead logging everywhere", None, &day(1)),
            Some(&vectors([1.0, 0.0, 0.0])),
        )
        .unwrap();
        let filter = Filter::default();
        let results = search(
            &db,
            &request("journal mode", Some(&[0.9, 0.1, 0.0]), &filter),
        )
        .unwrap();
        assert_eq!(contents(&results), ["write-ahead logging everywhere"]);
    }

    #[test]
    fn matching_both_arms_ranks_first_and_scores_descend() {
        let mut db = Database::open_in_memory().unwrap();
        // Opposite vector: beyond max_vector_distance, so this one matches text only.
        db.insert_memory(
            &record("wal only in text", None, &day(1)),
            Some(&vectors([-1.0, 0.0, 0.0])),
        )
        .unwrap();
        db.insert_memory(
            &record("wal in text and vector", None, &day(1)),
            Some(&vectors([1.0, 0.0, 0.0])),
        )
        .unwrap();
        let filter = Filter::default();
        let results = search(&db, &request("wal", Some(&[1.0, 0.0, 0.0]), &filter)).unwrap();
        assert_eq!(results[0].memory.content, "wal in text and vector");
        assert!(
            results
                .windows(2)
                .all(|pair| pair[0].score >= pair[1].score)
        );
        assert!(results.iter().all(|r| r.score > 0.0));
    }

    #[test]
    fn each_arm_looks_beyond_the_limit_before_merging() {
        let mut db = Database::open_in_memory().unwrap();
        // First in full text; its vector lies beyond max_vector_distance.
        db.insert_memory(
            &record("wal wal wal", None, &day(1)),
            Some(&vectors([-1.0, 0.0, 0.0])),
        )
        .unwrap();
        // Second in both arms.
        db.insert_memory(
            &record("wal among other words", None, &day(1)),
            Some(&vectors([0.9, 0.3, 0.0])),
        )
        .unwrap();
        // First by vector; no shared word.
        db.insert_memory(
            &record("journal mode", None, &day(1)),
            Some(&vectors([1.0, 0.0, 0.0])),
        )
        .unwrap();
        let filter = Filter::default();
        let top = SearchRequest {
            limit: 1,
            ..request("wal", Some(&[1.0, 0.0, 0.0]), &filter)
        };
        assert_eq!(
            contents(&search(&db, &top).unwrap()),
            ["wal among other words"]
        );
    }

    #[test]
    fn the_limit_applies_after_merging() {
        let mut db = Database::open_in_memory().unwrap();
        for n in 1..=5 {
            db.insert_memory(&record(&format!("wal note {n}"), None, &day(n)), None)
                .unwrap();
        }
        let filter = Filter::default();
        let limited = SearchRequest {
            limit: 2,
            ..request("wal", None, &filter)
        };
        assert_eq!(search(&db, &limited).unwrap().len(), 2);
    }

    #[test]
    fn the_largest_limit_returns_every_match() {
        let mut db = Database::open_in_memory().unwrap();
        for n in 1..=3 {
            db.insert_memory(&record(&format!("wal note {n}"), None, &day(n)), None)
                .unwrap();
        }
        let filter = Filter::default();
        let unbounded = SearchRequest {
            limit: usize::MAX,
            ..request("wal", None, &filter)
        };
        assert_eq!(search(&db, &unbounded).unwrap().len(), 3);
    }

    #[test]
    fn recency_prefers_newer_memories_when_enabled() {
        let mut db = Database::open_in_memory().unwrap();
        // The old memory wins on text alone (the term appears twice).
        db.insert_memory(&record("deploy checklist deploy", None, &day(1)), None)
            .unwrap();
        db.insert_memory(
            &NewRecord {
                global_id: "new".into(),
                ..record("deploy checklist", None, &day(29))
            },
            None,
        )
        .unwrap();
        let filter = Filter::default();
        let plain = search(&db, &request("deploy", None, &filter)).unwrap();
        assert_eq!(plain[0].memory.created_at, day(1));
        let aging = SearchRequest {
            recency: RecencyConfig {
                aging_factor: 1.0,
                half_life_days: 7.0,
            },
            ..request("deploy", None, &filter)
        };
        let results = search(&db, &aging).unwrap();
        assert_eq!(results[0].memory.created_at, day(29));
    }

    #[test]
    fn a_query_without_terms_is_rejected() {
        let db = Database::open_in_memory().unwrap();
        let filter = Filter::default();
        for query in ["   ", "***"] {
            assert!(matches!(
                search(&db, &request(query, None, &filter)),
                Err(Error::EmptyQuery)
            ));
        }
    }
}
