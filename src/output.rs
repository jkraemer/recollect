//! Text rendering of command results; `--json` output goes through serde.

use std::io::Write;

use crate::memory::{GLOBAL, Memory};
use crate::service::{Context, Status};

/// Writes one line to stderr, ignoring a stderr nobody reads (a closed pipe):
/// a diagnostic must not stop a command that can still do its work.
pub fn print_diagnostic(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// `#42 · project · type · date · tags` on one line, then the full content.
pub fn memory_block(memory: &Memory) -> String {
    let date = memory.created_at.get(..10).unwrap_or(&memory.created_at);
    let mut header = format!(
        "#{} · {} · {} · {date}",
        memory.id,
        memory.project.as_deref().unwrap_or(GLOBAL),
        memory.memory_type.as_str()
    );
    if !memory.tags.is_empty() {
        header.push_str(" · ");
        header.push_str(&memory.tags.join(", "));
    }
    format!("{header}\n{}", memory.content)
}

/// Memory blocks separated by a blank line; empty for no memories.
pub fn memory_blocks<'a>(memories: impl IntoIterator<Item = &'a Memory>) -> String {
    memories
        .into_iter()
        .map(memory_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn context_text(context: &Context) -> String {
    match context {
        Context::Project {
            last_session,
            recent_notes_todos,
            ..
        } => [
            section("Last session", last_session.iter()),
            section("Recent notes and todos", recent_notes_todos),
        ]
        .join("\n\n"),
        Context::AllProjects {
            recent_sessions,
            recent_notes_todos,
            ..
        } => [
            section("Recent sessions", recent_sessions),
            section("Recent notes and todos", recent_notes_todos),
        ]
        .join("\n\n"),
    }
}

fn section<'a>(heading: &str, memories: impl IntoIterator<Item = &'a Memory>) -> String {
    let body = memory_blocks(memories);
    if body.is_empty() {
        format!("{heading}\n(none)")
    } else {
        format!("{heading}\n{body}")
    }
}

/// Name and count per line, names padded to a common width.
pub fn counts_text(rows: &[(&str, usize)]) -> String {
    let width = rows
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(name, count)| format!("{name:<width$}  {count}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn status_text(status: &Status) -> String {
    let vectors = match &status.vectors_reason {
        None => "usable".to_string(),
        Some(reason) => format!("unusable: {reason}"),
    };
    let rows = [
        ("data dir", status.data_dir.clone()),
        ("database", status.database.clone()),
        ("memories", status.memories.to_string()),
        ("projects", status.projects.to_string()),
        ("embedding model", status.embedding_model.clone()),
        (
            "stored vectors from",
            status
                .stored_embedding_model
                .clone()
                .unwrap_or_else(|| "none".to_string()),
        ),
        ("vectors", vectors),
        ("pending embeddings", status.pending_embeddings.to_string()),
    ];
    let width = rows
        .iter()
        .map(|(label, _)| label.len() + 1)
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|(label, value)| format!("{:<width$} {value}", format!("{label}:")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryType;

    fn memory(id: i64, project: Option<&str>, tags: &[&str]) -> Memory {
        Memory {
            id,
            global_id: "g".into(),
            project: project.map(String::from),
            memory_type: MemoryType::Note,
            content: "Single SQLite DB.\nSecond line.".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: "2026-09-29T08:05:03.045Z".into(),
        }
    }

    #[test]
    fn a_memory_is_a_header_line_and_its_full_content() {
        assert_eq!(
            memory_block(&memory(42, Some("recollect"), &["decision", "sync"])),
            "#42 · recollect · note · 2026-09-29 · decision, sync\nSingle SQLite DB.\nSecond line."
        );
        assert_eq!(
            memory_block(&memory(7, None, &[])),
            "#7 · global · note · 2026-09-29\nSingle SQLite DB.\nSecond line."
        );
    }

    #[test]
    fn memories_are_separated_by_a_blank_line() {
        let memories = [memory(1, None, &[]), memory(2, None, &[])];
        let text = memory_blocks(&memories);
        assert_eq!(text.matches("\n\n#").count(), 1);
        assert_eq!(memory_blocks(&[]), "");
    }

    #[test]
    fn empty_context_sections_say_none() {
        let context = Context::Project {
            project: "p".into(),
            last_session: None,
            recent_notes_todos: vec![],
        };
        assert_eq!(
            context_text(&context),
            "Last session\n(none)\n\nRecent notes and todos\n(none)"
        );
        let all = Context::AllProjects {
            project: None,
            recent_sessions: vec![],
            recent_notes_todos: vec![memory(3, None, &[])],
        };
        assert_eq!(
            context_text(&all),
            "Recent sessions\n(none)\n\nRecent notes and todos\n#3 · global · note · 2026-09-29\nSingle SQLite DB.\nSecond line."
        );
    }

    #[test]
    fn counts_are_aligned_in_two_columns() {
        assert_eq!(
            counts_text(&[("recollect", 12), ("global", 3)]),
            "recollect  12\nglobal     3"
        );
        assert_eq!(counts_text(&[]), "");
    }

    #[test]
    fn status_lists_every_field() {
        let status = Status {
            data_dir: "/d".into(),
            database: "/d/memories.db".into(),
            memories: 3,
            projects: 2,
            embedding_model: "bge-small-en-v1.5-q".into(),
            stored_embedding_model: None,
            vectors_usable: false,
            vectors_reason: Some("not downloaded".into()),
            pending_embeddings: 3,
        };
        assert_eq!(
            status_text(&status),
            "data dir:            /d\n\
             database:            /d/memories.db\n\
             memories:            3\n\
             projects:            2\n\
             embedding model:     bge-small-en-v1.5-q\n\
             stored vectors from: none\n\
             vectors:             unusable: not downloaded\n\
             pending embeddings:  3"
        );
    }
}
