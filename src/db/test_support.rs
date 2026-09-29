//! Record builders shared by the storage and search unit tests.

use crate::memory::{MemoryType, NewRecord};

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
