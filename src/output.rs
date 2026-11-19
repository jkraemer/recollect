//! Text rendering of command results; `--json` output goes through serde.

use std::io::Write;

use crate::db::{Peer, SyncApplied};
use crate::error::Result;
use crate::memory::{GLOBAL, Memory, Reference};
use crate::service::{Context, Shown, Status};
use crate::sync::LocalMachine;
use crate::sync::round::RoundOutcome;

/// Writes one line to stderr, ignoring a stderr nobody reads (a closed pipe):
/// a diagnostic must not stop a command that can still do its work.
pub fn print_diagnostic(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// `#42 · project · type · date · tags`, and `· superseded` when a live
/// memory supersedes it.
fn memory_header(memory: &Memory) -> String {
    let mut header = format!(
        "#{} · {} · {} · {}",
        memory.id,
        memory.project.as_deref().unwrap_or(GLOBAL),
        memory.memory_type.as_str(),
        memory.date()
    );
    if !memory.tags.is_empty() {
        header.push_str(" · ");
        header.push_str(&memory.tags.join(", "));
    }
    if !memory.superseded_by.is_empty() {
        header.push_str(" · superseded");
    }
    header
}

/// The header on one line, then the full content.
pub fn memory_block(memory: &Memory) -> String {
    format!("{}\n{}", memory_header(memory), memory.content)
}

/// A memory block with the lines naming what it supersedes and what
/// supersedes it between header and content.
pub fn shown_text(shown: &Shown) -> String {
    let mut lines = vec![memory_header(&shown.memory)];
    for (label, references) in [
        ("supersedes", &shown.supersedes),
        ("superseded by", &shown.superseded_by),
    ] {
        if !references.is_empty() {
            let refs: Vec<String> = references.iter().map(reference_text).collect();
            lines.push(format!("{label} {}", refs.join(", ")));
        }
    }
    lines.push(shown.memory.content.clone());
    lines.join("\n")
}

/// `#12`, `#12 (deleted)`, or the global id of a memory this machine lacks.
fn reference_text(reference: &Reference) -> String {
    match (reference.id, reference.deleted) {
        (Some(id), false) => format!("#{id}"),
        (Some(id), true) => format!("#{id} (deleted)"),
        (None, _) => format!("{} (not on this machine)", reference.global_id),
    }
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
        ("superseded", status.superseded.to_string()),
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
    let forks = status.forks.iter().map(|fork| {
        let superseders: Vec<String> = fork.superseded_by.iter().map(reference_text).collect();
        (
            "fork",
            format!(
                "{} superseded by {}",
                reference_text(&fork.target),
                superseders.join(", ")
            ),
        )
    });
    labelled(&rows.into_iter().chain(forks).collect::<Vec<_>>())
}

/// Rows of `label: value`, the values aligned.
fn labelled(rows: &[(&str, String)]) -> String {
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

pub fn machine_text(machine: &LocalMachine) -> String {
    labelled(&[
        ("name", machine.name.clone()),
        ("fingerprint", machine.fingerprint.clone()),
        ("listen", machine.listen.clone()),
    ])
}

/// One line per peer: name, address, fingerprint and last sync; the reason
/// of a failed last round goes on a line of its own below.
pub fn peers_text(peers: &[Peer]) -> String {
    peers
        .iter()
        .map(|peer| {
            let synced = match &peer.last_sync_at {
                Some(at) => format!("last synced {at}"),
                None => "never synced".to_string(),
            };
            let line = format!(
                "{} · {} · {} · {synced}",
                peer.name, peer.address, peer.fingerprint
            );
            match &peer.last_error {
                Some(error) => format!("{line}\n  error: {error}"),
                None => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `3 memories and 1 deletion`; `None` when nothing was applied.
fn changes_text(applied: &SyncApplied) -> Option<String> {
    let counted = |count: usize, one: &str, many: &str| match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        _ => Some(format!("{count} {many}")),
    };
    let parts: Vec<String> = [
        counted(applied.memories, "memory", "memories"),
        counted(applied.deletions, "deletion", "deletions"),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" and "))
}

/// One line saying how a round with `peer` went.
pub fn round_line(peer: &str, result: &Result<RoundOutcome>) -> String {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(err) => return format!("{peer}: error: {err}"),
    };
    let parts: Vec<String> = [("received", &outcome.received), ("sent", &outcome.sent)]
        .into_iter()
        .filter_map(|(verb, applied)| Some(format!("{verb} {}", changes_text(applied)?)))
        .collect();
    if parts.is_empty() {
        format!("{peer}: in sync")
    } else {
        format!("{peer}: {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Peer, SyncApplied};
    use crate::error::Error;
    use crate::memory::{Fork, MemoryType, Reference};
    use crate::service::Shown;
    use crate::sync::LocalMachine;
    use crate::sync::round::RoundOutcome;

    fn memory(id: i64, project: Option<&str>, tags: &[&str]) -> Memory {
        Memory {
            id,
            global_id: "g".into(),
            project: project.map(String::from),
            memory_type: MemoryType::Note,
            content: "Single SQLite DB.\nSecond line.".into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: "2026-09-29T08:05:03.045Z".into(),
            supersedes: Vec::new(),
            superseded_by: Vec::new(),
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
            superseded: 0,
            forks: Vec::new(),
        };
        assert_eq!(
            status_text(&status),
            "data dir:            /d\n\
             database:            /d/memories.db\n\
             memories:            3\n\
             superseded:          0\n\
             projects:            2\n\
             embedding model:     bge-small-en-v1.5-q\n\
             stored vectors from: none\n\
             vectors:             unusable: not downloaded\n\
             pending embeddings:  3"
        );
    }

    #[test]
    fn status_lists_each_fork_on_a_row() {
        let status = Status {
            data_dir: "/d".into(),
            database: "/d/memories.db".into(),
            memories: 3,
            projects: 2,
            embedding_model: "m".into(),
            stored_embedding_model: None,
            vectors_usable: true,
            vectors_reason: None,
            pending_embeddings: 0,
            superseded: 1,
            forks: vec![Fork {
                target: reference("a", Some(1), false),
                superseded_by: vec![
                    reference("b", Some(2), false),
                    reference("c", Some(3), false),
                ],
            }],
        };
        assert!(
            status_text(&status).ends_with("\nfork:                #1 superseded by #2, #3"),
            "{}",
            status_text(&status)
        );
    }

    fn reference(global_id: &str, id: Option<i64>, deleted: bool) -> Reference {
        Reference {
            global_id: global_id.into(),
            id,
            deleted,
        }
    }

    #[test]
    fn a_superseded_memory_says_so_after_its_tags() {
        let mut superseded = memory(1, None, &["x"]);
        superseded.superseded_by = vec!["g2".into()];
        assert_eq!(
            memory_block(&superseded),
            "#1 · global · note · 2026-09-29 · x · superseded\nSingle SQLite DB.\nSecond line."
        );
        superseded.tags.clear();
        assert!(
            memory_block(&superseded).starts_with("#1 · global · note · 2026-09-29 · superseded\n")
        );
    }

    #[test]
    fn a_shown_memory_names_what_it_supersedes_and_what_supersedes_it() {
        let shown = Shown {
            memory: memory(5, None, &[]),
            supersedes: vec![
                reference("a", Some(1), false),
                reference("b", Some(2), true),
                reference("0199a8c0-0000-7000-8000-000000000001", None, false),
            ],
            superseded_by: vec![reference("c", Some(9), false)],
        };
        assert_eq!(
            shown_text(&shown),
            "#5 · global · note · 2026-09-29\n\
             supersedes #1, #2 (deleted), 0199a8c0-0000-7000-8000-000000000001 (not on this machine)\n\
             superseded by #9\n\
             Single SQLite DB.\nSecond line."
        );
        let plain = Shown {
            memory: memory(5, None, &[]),
            supersedes: vec![],
            superseded_by: vec![],
        };
        assert_eq!(shown_text(&plain), memory_block(&plain.memory));
    }

    fn outcome(
        received: (usize, usize),
        sent: (usize, usize),
    ) -> crate::error::Result<RoundOutcome> {
        let applied = |(memories, deletions)| SyncApplied {
            memories,
            deletions,
        };
        Ok(RoundOutcome {
            received: applied(received),
            sent: applied(sent),
        })
    }

    #[test]
    fn a_round_is_one_line_saying_what_moved() {
        for (result, expected) in [
            (outcome((0, 0), (0, 0)), "twelve: in sync"),
            (
                outcome((3, 1), (2, 0)),
                "twelve: received 3 memories and 1 deletion, sent 2 memories",
            ),
            (outcome((1, 0), (0, 0)), "twelve: received 1 memory"),
            (outcome((0, 0), (0, 2)), "twelve: sent 2 deletions"),
            (
                Err(Error::Sync("twelve:7327: cannot connect (refused)".into())),
                "twelve: error: twelve:7327: cannot connect (refused)",
            ),
        ] {
            assert_eq!(round_line("twelve", &result), expected);
        }
    }

    #[test]
    fn the_machine_is_shown_as_aligned_rows() {
        let machine = LocalMachine {
            name: "foehn".into(),
            fingerprint: "SHA256:abc".into(),
            listen: "0.0.0.0:7327".into(),
        };
        assert_eq!(
            machine_text(&machine),
            "name:        foehn\nfingerprint: SHA256:abc\nlisten:      0.0.0.0:7327"
        );
    }

    #[test]
    fn peers_are_one_line_each_with_the_last_failure_below() {
        let peer = |name: &str, last_sync_at: Option<&str>, last_error: Option<&str>| Peer {
            name: name.into(),
            fingerprint: format!("SHA256:{name}"),
            address: format!("{name}:7327"),
            added_at: "2026-10-07T10:00:00.000Z".into(),
            last_sync_at: last_sync_at.map(String::from),
            last_error: last_error.map(String::from),
        };
        let peers = [
            peer("foehn", Some("2026-10-07T11:00:00.000Z"), None),
            peer(
                "twelve",
                None,
                Some("twelve:7327: cannot connect (refused)"),
            ),
        ];
        assert_eq!(
            peers_text(&peers),
            "foehn · foehn:7327 · SHA256:foehn · last synced 2026-10-07T11:00:00.000Z\n\
             twelve · twelve:7327 · SHA256:twelve · never synced\n  \
             error: twelve:7327: cannot connect (refused)"
        );
        assert_eq!(peers_text(&[]), "");
    }
}
