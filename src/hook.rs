//! Claude Code hooks: their JSON input and the context a session starts with.

use std::path::PathBuf;

use serde::Deserialize;

use crate::detect::{Detection, ProjectSource};
use crate::error::{Error, Result};
use crate::memory::{GLOBAL, Memory};
use crate::service::Context;

/// The most characters `session_start_text` returns. Claude Code puts at most
/// 10,000 characters of hook output into context and replaces longer output
/// with a short preview.
pub const SESSION_START_BUDGET: usize = 9_000;

/// How many characters of a memory's first line an index line shows.
const INDEX_TEXT_CHARS: usize = 100;

/// The tag of the session memories made from Claude Code's compaction summaries.
pub const COMPACTION_TAG: &str = "compaction";

const BLOCK_SEPARATOR: &str = "\n\n";

/// The fields of Claude Code's hook input that recollect uses; the others
/// are ignored, so new fields in later Claude Code versions do no harm.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    /// The session's working directory.
    pub cwd: Option<PathBuf>,
    /// The summary Claude Code just wrote; only in `PostCompact` input.
    pub compact_summary: Option<String>,
}

impl HookInput {
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|err| Error::HookInput(err.to_string()))
    }
}

/// The markdown a session starts with: the project of the session directory
/// and how it was found, the commands for it, and its context
/// (`Context::Project`): the last session in full and an index of the recent
/// notes and todos. Without a project, the reason and an index of the recent
/// sessions and notes and todos everywhere (`Context::AllProjects`). At most
/// `SESSION_START_BUDGET` characters: the header and the index are sized
/// first, and a last session longer than the rest is cut.
pub fn session_start_text(detection: &Detection, context: &Context) -> String {
    let mut blocks = vec![header(detection)];
    match context {
        Context::Project {
            last_session,
            recent_notes_todos,
            ..
        } => {
            let notes = index_section("Recent notes and todos", recent_notes_todos, false);
            if let Some(session) = last_session {
                let taken: usize = blocks
                    .iter()
                    .chain(&notes)
                    .map(|block| chars(block) + BLOCK_SEPARATOR.len())
                    .sum();
                blocks.push(session_section(
                    session,
                    SESSION_START_BUDGET.saturating_sub(taken),
                ));
            }
            blocks.extend(notes);
            if blocks.len() == 1 {
                blocks.push("No memories stored for this project yet.".to_string());
            }
        }
        Context::AllProjects {
            recent_sessions,
            recent_notes_todos,
            ..
        } => {
            blocks.extend(index_section("Recent sessions", recent_sessions, true));
            blocks.extend(index_section(
                "Recent notes and todos",
                recent_notes_todos,
                true,
            ));
            if blocks.len() == 1 {
                blocks.push("No memories stored yet.".to_string());
            }
        }
    }
    blocks.join(BLOCK_SEPARATOR)
}

/// The title, how the project was found or why there is none, and the commands.
fn header(detection: &Detection) -> String {
    match detection {
        Detection::Found { project, source } => {
            let name = project.display_name();
            let origin = match source {
                ProjectSource::Repository(dir) => {
                    format!("Project from the repository directory {}.", dir.display())
                }
                ProjectSource::ProjectFile(file) => format!("Project from {}.", file.display()),
            };
            format!(
                "# Recollect memory: project {name}\n\n{origin}\n{}",
                commands("Commands for this project", name)
            )
        }
        Detection::NotFound { reason } => format!(
            "# Recollect memory: no project\n\n{reason}\n{}",
            commands(
                "Commands, with the project name in place of <project>",
                "<project>"
            )
        ),
    }
}

fn commands(label: &str, project: &str) -> String {
    format!(
        "{label}: search `recollect search \"<words>\" -p {project} --json`, \
         full text `recollect show <id>`, \
         store `recollect store -p {project} -T <tags>` with the content on stdin."
    )
}

/// The last session under its heading; longer than `room` characters, cut at
/// a line boundary and followed by a pointer to the full text.
fn session_section(session: &Memory, room: usize) -> String {
    let mut heading = format!("## Last session · #{} · {}", session.id, session.date());
    if !session.tags.is_empty() {
        heading.push_str(" · ");
        heading.push_str(&session.tags.join(", "));
    }
    let full = format!("{heading}\n{}", session.content);
    if chars(&full) <= room {
        return full;
    }
    let pointer = format!("[… cut; recollect show {} for the rest]", session.id);
    // The two newlines: after the heading and before the pointer.
    let content_room = room.saturating_sub(chars(&heading) + chars(&pointer) + 2);
    format!(
        "{heading}\n{}\n{pointer}",
        cut_at_line(&session.content, content_room)
    )
}

/// The longest start of `text` of at most `max` characters that ends at a
/// line boundary; mid-line when not even the first line fits.
fn cut_at_line(text: &str, max: usize) -> &str {
    let end = text
        .char_indices()
        .nth(max)
        .map_or(text.len(), |(index, _)| index);
    let (kept, rest) = text.split_at(end);
    if rest.is_empty() || rest.starts_with('\n') {
        return kept;
    }
    match kept.rfind('\n') {
        Some(newline) => &kept[..newline],
        None => kept,
    }
}

/// `## <heading>` over one index line per memory; nothing for no memories.
fn index_section(heading: &str, memories: &[Memory], with_project: bool) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let lines: Vec<String> = memories
        .iter()
        .map(|memory| index_line(memory, with_project))
        .collect();
    Some(format!("## {heading}\n{}", lines.join("\n")))
}

/// `- #<id> · [<project> · ]<type> · <date> · [<tags> · ]<first line>`.
fn index_line(memory: &Memory, with_project: bool) -> String {
    let mut fields = vec![format!("#{}", memory.id)];
    if with_project {
        fields.push(memory.project.as_deref().unwrap_or(GLOBAL).to_string());
    }
    fields.push(memory.memory_type.as_str().to_string());
    fields.push(memory.date().to_string());
    if !memory.tags.is_empty() {
        fields.push(memory.tags.join(", "));
    }
    fields.push(first_line(&memory.content));
    format!("- {}", fields.join(" · "))
}

/// The first non-empty line of `content`, trimmed; when it is longer than
/// `INDEX_TEXT_CHARS` characters, its start followed by `…`.
fn first_line(content: &str) -> String {
    let line = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    match line.char_indices().nth(INDEX_TEXT_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_string(),
    }
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, ProjectRef};

    fn memory(
        id: i64,
        project: Option<&str>,
        memory_type: MemoryType,
        content: &str,
        tags: &[&str],
    ) -> Memory {
        Memory {
            id,
            global_id: format!("g-{id}"),
            project: project.map(String::from),
            memory_type,
            content: content.to_string(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            created_at: "2026-05-02T10:00:00.000Z".into(),
        }
    }

    fn fera() -> Detection {
        Detection::Found {
            project: ProjectRef::Named("fera".into()),
            source: ProjectSource::Repository(PathBuf::from("/work/fera")),
        }
    }

    fn fera_context(last_session: Option<Memory>, recent_notes_todos: Vec<Memory>) -> Context {
        Context::Project {
            project: "fera".into(),
            last_session,
            recent_notes_todos,
        }
    }

    /// Ten notes, as many as a project's context holds.
    fn ten_notes() -> Vec<Memory> {
        (1..=10)
            .map(|id| {
                let content = format!("note number {id} {}", "x".repeat(120));
                memory(id, Some("fera"), MemoryType::Note, &content, &["tag"])
            })
            .collect()
    }

    const FERA_HEADER: &str = "# Recollect memory: project fera\n\n\
        Project from the repository directory /work/fera.\n\
        Commands for this project: search `recollect search \"<words>\" -p fera --json`, \
        full text `recollect show <id>`, \
        store `recollect store -p fera -T <tags>` with the content on stdin.";

    #[test]
    fn a_project_context_is_the_header_the_last_session_and_the_notes_index() {
        let context = fera_context(
            Some(memory(
                452,
                Some("fera"),
                MemoryType::Session,
                "Session: Billing\n\nWe split invoices.",
                &["billing"],
            )),
            vec![
                memory(
                    451,
                    Some("fera"),
                    MemoryType::Todo,
                    "Check the VAT rounding",
                    &[],
                ),
                memory(
                    450,
                    Some("fera"),
                    MemoryType::Note,
                    "\n  Invoices are immutable once sent.  \nDetails follow.",
                    &["billing", "decision"],
                ),
            ],
        );
        assert_eq!(
            session_start_text(&fera(), &context),
            format!(
                "{FERA_HEADER}\n\n\
                 ## Last session · #452 · 2026-05-02 · billing\n\
                 Session: Billing\n\nWe split invoices.\n\n\
                 ## Recent notes and todos\n\
                 - #451 · todo · 2026-05-02 · Check the VAT rounding\n\
                 - #450 · note · 2026-05-02 · billing, decision · Invoices are immutable once sent."
            )
        );
    }

    #[test]
    fn index_lines_show_the_first_100_characters_of_the_first_line() {
        let long = format!("{}äb", "a".repeat(99));
        let exact = "b".repeat(100);
        let context = fera_context(
            None,
            vec![
                memory(
                    2,
                    Some("fera"),
                    MemoryType::Note,
                    &format!("{long}\nsecond line"),
                    &[],
                ),
                memory(1, Some("fera"), MemoryType::Note, &exact, &[]),
            ],
        );
        let text = session_start_text(&fera(), &context);
        assert!(
            text.ends_with(&format!(
                "\n- #2 · note · 2026-05-02 · {}ä…\n- #1 · note · 2026-05-02 · {exact}",
                "a".repeat(99)
            )),
            "{text}"
        );
    }

    #[test]
    fn sections_without_memories_are_left_out() {
        let note = memory(1, Some("fera"), MemoryType::Note, "n", &[]);
        assert_eq!(
            session_start_text(&fera(), &fera_context(None, vec![note])),
            format!("{FERA_HEADER}\n\n## Recent notes and todos\n- #1 · note · 2026-05-02 · n")
        );
        let session = memory(2, Some("fera"), MemoryType::Session, "s", &[]);
        assert_eq!(
            session_start_text(&fera(), &fera_context(Some(session), vec![])),
            format!("{FERA_HEADER}\n\n## Last session · #2 · 2026-05-02\ns")
        );
        assert_eq!(
            session_start_text(&fera(), &fera_context(None, vec![])),
            format!("{FERA_HEADER}\n\nNo memories stored for this project yet.")
        );
    }

    #[test]
    fn a_long_last_session_is_cut_at_a_line_boundary_to_fit_the_budget() {
        let lines: Vec<String> = (1..=400)
            .map(|n| format!("line {n:03} {}", "y".repeat(40)))
            .collect();
        let session = memory(
            77,
            Some("fera"),
            MemoryType::Session,
            &lines.join("\n"),
            &[],
        );
        let text = session_start_text(&fera(), &fera_context(Some(session), ten_notes()));
        let length = text.chars().count();
        assert!(length <= SESSION_START_BUDGET, "{length} characters");
        // A cut at a line boundary leaves at most one 49-character line and its newline unused.
        assert!(length >= SESSION_START_BUDGET - 50, "{length} characters");
        let (session, notes) = text
            .split_once("\n[… cut; recollect show 77 for the rest]\n\n")
            .expect("the cut session ends with the pointer");
        let last_kept = session.lines().last().unwrap();
        assert!(
            lines.iter().any(|line| line == last_kept),
            "cut at a line boundary, got {last_kept:?}"
        );
        assert_eq!(
            notes.lines().filter(|line| line.starts_with("- #")).count(),
            10,
            "the notes index is complete"
        );
    }

    #[test]
    fn a_first_line_longer_than_the_budget_is_cut_mid_line() {
        let session = memory(
            5,
            Some("fera"),
            MemoryType::Session,
            &"z".repeat(20_000),
            &[],
        );
        let text = session_start_text(&fera(), &fera_context(Some(session), vec![]));
        assert_eq!(text.chars().count(), SESSION_START_BUDGET);
        assert!(
            text.ends_with("\n[… cut; recollect show 5 for the rest]"),
            "{text}"
        );
        let kept = text.lines().rev().nth(1).unwrap();
        assert!(
            !kept.is_empty() && kept.chars().all(|c| c == 'z'),
            "{kept:?}"
        );
    }

    #[test]
    fn multi_byte_text_is_measured_and_cut_in_characters() {
        let lines: Vec<String> = (0..600)
            .map(|n| format!("{n}: Grüße · naïve … 🦀 {}", "ä".repeat(20)))
            .collect();
        let session = memory(
            9,
            Some("fera"),
            MemoryType::Session,
            &lines.join("\n"),
            &["ünïcode"],
        );
        let text = session_start_text(&fera(), &fera_context(Some(session), ten_notes()));
        assert!(text.chars().count() <= SESSION_START_BUDGET);
        assert!(
            text.len() > SESSION_START_BUDGET,
            "more bytes than characters, so the budget counts characters"
        );
        assert!(
            text.contains(
                "\n[… cut; recollect show 9 for the rest]\n\n## Recent notes and todos\n"
            )
        );
    }

    #[test]
    fn without_a_project_the_reason_and_the_recent_memories_everywhere_are_shown() {
        let detection = Detection::NotFound {
            reason: "/tmp/x is not in a git repository and has no .recollect-project file.".into(),
        };
        let context = Context::AllProjects {
            project: None,
            recent_sessions: vec![memory(
                8,
                Some("fera"),
                MemoryType::Session,
                "Session: Billing\nmore",
                &["billing"],
            )],
            recent_notes_todos: vec![memory(7, None, MemoryType::Note, "A global note", &[])],
        };
        assert_eq!(
            session_start_text(&detection, &context),
            "# Recollect memory: no project\n\n\
             /tmp/x is not in a git repository and has no .recollect-project file.\n\
             Commands, with the project name in place of <project>: \
             search `recollect search \"<words>\" -p <project> --json`, \
             full text `recollect show <id>`, \
             store `recollect store -p <project> -T <tags>` with the content on stdin.\n\n\
             ## Recent sessions\n\
             - #8 · fera · session · 2026-05-02 · billing · Session: Billing\n\n\
             ## Recent notes and todos\n\
             - #7 · global · note · 2026-05-02 · A global note"
        );
        let nothing = Context::AllProjects {
            project: None,
            recent_sessions: vec![],
            recent_notes_todos: vec![],
        };
        assert!(
            session_start_text(&detection, &nothing)
                .ends_with("with the content on stdin.\n\nNo memories stored yet.")
        );
    }

    #[test]
    fn a_project_named_by_a_project_file_says_so() {
        let detection = Detection::Found {
            project: ProjectRef::Named("web-app".into()),
            source: ProjectSource::ProjectFile(PathBuf::from("/work/mono/web/.recollect-project")),
        };
        let context = Context::Project {
            project: "web-app".into(),
            last_session: None,
            recent_notes_todos: vec![],
        };
        let text = session_start_text(&detection, &context);
        assert!(
            text.starts_with(
                "# Recollect memory: project web-app\n\n\
                 Project from /work/mono/web/.recollect-project.\n\
                 Commands for this project: search `recollect search \"<words>\" -p web-app --json`"
            ),
            "{text}"
        );
    }

    #[test]
    fn hook_input_takes_cwd_and_ignores_fields_it_does_not_use() {
        let start = HookInput::parse(
            r#"{"session_id":"abc123","transcript_path":"/home/u/.claude/projects/fera/abc123.jsonl","cwd":"/work/fera","hook_event_name":"SessionStart","source":"startup","model":"claude-opus-5-5","permission_mode":"default"}"#,
        )
        .unwrap();
        assert_eq!(start.cwd, Some(PathBuf::from("/work/fera")));
        assert_eq!(start.compact_summary, None);
        let compact = HookInput::parse(
            r#"{"cwd":"/work/fera","hook_event_name":"PostCompact","trigger":"auto","compact_summary":"We did things."}"#,
        )
        .unwrap();
        assert_eq!(compact.compact_summary.as_deref(), Some("We did things."));
        assert_eq!(HookInput::parse(r#"{"cwd":null}"#).unwrap().cwd, None);
        assert_eq!(HookInput::parse("{}").unwrap().cwd, None);
    }

    #[test]
    fn malformed_hook_input_is_an_error() {
        for text in ["", "not json", "[]", r#"{"cwd": 42}"#] {
            let err = HookInput::parse(text).unwrap_err();
            assert!(matches!(err, Error::HookInput(_)), "{text:?}: {err}");
            assert!(err.to_string().starts_with("invalid hook input: "), "{err}");
        }
    }
}
