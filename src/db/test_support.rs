//! Record builders and probes shared by the storage and search unit tests.

use super::Database;
use crate::memory::{MemoryType, NewRecord};

/// When the memories of the storage tests are created.
pub const T0: &str = "2026-09-01T10:00:00.000Z";

/// A day after `T0`: when they are deleted.
pub const LATER: &str = "2026-09-02T10:00:00.000Z";

/// A live note created at `created_at`; its global id derives from `content`.
pub fn record(content: &str, project: Option<&str>, created_at: &str) -> NewRecord {
    NewRecord {
        global_id: format!("test-{content}"),
        project: project.map(String::from),
        memory_type: MemoryType::Note,
        content: content.to_string(),
        tags: Vec::new(),
        origin_peer: None,
        created_at: created_at.to_string(),
    }
}

/// A live note in project `p` with a UUID as its global id, as sync requires.
pub fn note(content: &str) -> NewRecord {
    NewRecord {
        global_id: uuid::Uuid::now_v7().to_string(),
        ..record(content, Some("p"), T0)
    }
}

/// How many memories the full-text index finds for `query`.
pub fn fts_hits(db: &Database, query: &str) -> i64 {
    db.conn
        .query_row(
            "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH ?1",
            [query],
            |row| row.get(0),
        )
        .unwrap()
}

/// Runs `work(n)` for `n` in `0..threads`, all starting at the same moment,
/// and returns the results in order of `n`.
pub fn concurrently<T: Send>(threads: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let start = std::sync::Barrier::new(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|n| {
                let (start, work) = (&start, &work);
                scope.spawn(move || {
                    start.wait();
                    work(n)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    })
}
