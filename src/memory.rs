//! Memory domain types and the normalization rules shared by every command.

use serde::Serialize;

use crate::error::{Error, Result};

/// The CLI's name for memories without a project (`project IS NULL`).
pub const GLOBAL: &str = "global";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Note,
    Todo,
    Session,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::Note => "note",
            MemoryType::Todo => "todo",
            MemoryType::Session => "session",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "note" => Some(MemoryType::Note),
            "todo" => Some(MemoryType::Todo),
            "session" => Some(MemoryType::Session),
            _ => None,
        }
    }
}

/// A project as named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectRef {
    Global,
    Named(String),
}

impl ProjectRef {
    /// Trims and lowercases; `global` selects memories without a project.
    pub fn parse(raw: &str) -> Result<Self> {
        let name = raw.trim().to_lowercase();
        if name == GLOBAL {
            return Ok(ProjectRef::Global);
        }
        let allowed =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
        if !name.is_empty() && name.chars().all(allowed) {
            Ok(ProjectRef::Named(name))
        } else {
            Err(Error::InvalidProject(raw.to_string()))
        }
    }

    /// The value of the `project` column.
    pub fn column_value(&self) -> Option<&str> {
        match self {
            ProjectRef::Global => None,
            ProjectRef::Named(name) => Some(name),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            ProjectRef::Global => GLOBAL,
            ProjectRef::Named(name) => name,
        }
    }
}

/// A live memory as returned by every read command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Memory {
    pub id: i64,
    pub global_id: String,
    pub project: Option<String>,
    pub memory_type: MemoryType,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

impl Memory {
    /// The day the memory was created, `YYYY-MM-DD`.
    pub fn date(&self) -> &str {
        self.created_at.get(..10).unwrap_or(&self.created_at)
    }
}

/// A search result: the memory plus its final ranking score.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoredMemory {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f64,
}

/// Everything needed to insert a memory row. `store` generates `global_id` and
/// `created_at`; import and sync pass the original values.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRecord {
    pub global_id: String,
    pub project: Option<String>,
    pub memory_type: MemoryType,
    pub content: String,
    pub tags: Vec<String>,
    /// `None` means the memory was created on this machine.
    pub origin_peer: Option<String>,
    pub created_at: String,
}

/// A memory deleted elsewhere: its `global_id` and when it was deleted, in
/// the stored timestamp format.
#[derive(Debug, Clone, PartialEq)]
pub struct Tombstone {
    pub global_id: String,
    pub deleted_at: String,
}

/// Chunk embeddings of one memory, all produced by the model `model_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    pub model_id: String,
    pub chunks: Vec<Vec<f32>>,
}

/// Trims surrounding whitespace; empty content is rejected.
pub fn normalize_content(raw: &str) -> Result<String> {
    let content = raw.trim();
    if content.is_empty() {
        Err(Error::EmptyContent)
    } else {
        Ok(content.to_string())
    }
}

/// Trims and lowercases every tag, then sorts and deduplicates; empty tags are rejected.
pub fn normalize_tags(raw: &[String]) -> Result<Vec<String>> {
    let mut tags = Vec::with_capacity(raw.len());
    for tag in raw {
        let normalized = tag.trim().to_lowercase();
        if normalized.is_empty() {
            return Err(Error::InvalidTag(tag.clone()));
        }
        tags.push(normalized);
    }
    tags.sort();
    tags.dedup();
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn project_names_are_trimmed_and_lowercased() {
        assert_eq!(
            ProjectRef::parse("  Recollect ").unwrap(),
            ProjectRef::Named("recollect".into())
        );
        assert_eq!(
            ProjectRef::parse("my-proj_1.x").unwrap(),
            ProjectRef::Named("my-proj_1.x".into())
        );
    }

    #[test]
    fn global_names_the_memories_without_a_project() {
        let global = ProjectRef::parse("GLOBAL").unwrap();
        assert_eq!(global, ProjectRef::Global);
        assert_eq!(global.column_value(), None);
        assert_eq!(global.display_name(), "global");
        assert_eq!(ProjectRef::Named("x".into()).column_value(), Some("x"));
    }

    #[test]
    fn project_names_outside_the_allowed_set_are_rejected() {
        for raw in ["a b", "a/b", "", "   ", "ä"] {
            match ProjectRef::parse(raw) {
                Err(Error::InvalidProject(name)) => assert_eq!(name, raw),
                other => panic!("{raw:?} must be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn tags_are_trimmed_lowercased_deduplicated_and_sorted() {
        let raw = vec![" Sync".to_string(), "decision".into(), "sync".into()];
        assert_eq!(normalize_tags(&raw).unwrap(), vec!["decision", "sync"]);
    }

    #[test]
    fn empty_tags_are_rejected() {
        let raw = vec!["a".to_string(), "  ".into()];
        assert!(matches!(normalize_tags(&raw), Err(Error::InvalidTag(_))));
    }

    #[test]
    fn content_is_trimmed_and_must_not_be_empty() {
        assert_eq!(normalize_content("  x\n y \n").unwrap(), "x\n y");
        assert!(matches!(
            normalize_content(" \n\t"),
            Err(Error::EmptyContent)
        ));
    }

    #[test]
    fn memory_types_round_trip_through_their_database_names() {
        for memory_type in [MemoryType::Note, MemoryType::Todo, MemoryType::Session] {
            assert_eq!(MemoryType::from_db(memory_type.as_str()), Some(memory_type));
        }
        assert_eq!(MemoryType::from_db("_chunk"), None);
    }

    #[test]
    fn memories_serialize_with_a_null_project_for_global() {
        let memory = Memory {
            id: 7,
            global_id: "g".into(),
            project: None,
            memory_type: MemoryType::Todo,
            content: "c".into(),
            tags: vec!["a".into()],
            created_at: "2026-09-29T10:00:00.000Z".into(),
        };
        assert_eq!(
            serde_json::to_value(&memory).unwrap(),
            json!({
                "id": 7, "global_id": "g", "project": null, "memory_type": "todo",
                "content": "c", "tags": ["a"], "created_at": "2026-09-29T10:00:00.000Z"
            })
        );
        let scored = ScoredMemory { memory, score: 0.5 };
        let value = serde_json::to_value(&scored).unwrap();
        assert_eq!(value["score"], 0.5);
        assert_eq!(value["id"], 7);
    }
}
