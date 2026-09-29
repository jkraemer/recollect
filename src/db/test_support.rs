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
