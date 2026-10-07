//! The rules of an exchange, without any networking: what a manifest is,
//! what each side owes the other, what a peer may send, and how memories
//! are cut into messages.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::memory::{ProjectRef, SyncRecord, Tombstone, normalize_content, normalize_tags};
use crate::sync::protocol::{EntryState, ManifestEntry};
use crate::sync::sha256_hex;
use crate::time::parse_timestamp;

/// The most memories in one `records` message.
pub const RECORDS_PER_MESSAGE: usize = 200;

/// The content one `records` message may hold before the next memory starts
/// a new message; far below the message limit, so a message of large
/// memories stays under it. A single larger memory travels alone.
pub const CONTENT_BYTES_PER_MESSAGE: usize = 4 * 1024 * 1024;

/// The manifest of the rows `Database::sync_manifest` returns.
pub fn manifest(rows: Vec<(String, bool)>) -> Vec<ManifestEntry> {
    rows.into_iter()
        .map(|(global_id, deleted)| {
            let state = if deleted {
                EntryState::Deleted
            } else {
                EntryState::Live
            };
            (global_id, state)
        })
        .collect()
}

/// Identifies what a machine holds: the SHA-256 over the sorted lines
/// `<global_id> live|deleted`. Two machines with the same hash hold the same
/// memories and deletions.
pub fn manifest_hash(entries: &[ManifestEntry]) -> String {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|(global_id, state)| {
            let state = match state {
                EntryState::Live => "live",
                EntryState::Deleted => "deleted",
            };
            format!("{global_id} {state}\n")
        })
        .collect();
    lines.sort();
    sha256_hex(lines.concat().as_bytes())
}

/// What one side sends: the global ids of the memories to send in full, and
/// of those to send only the deletion of.
#[derive(Debug, PartialEq)]
pub struct Outgoing {
    pub records: Vec<String>,
    pub tombstones: Vec<String>,
}

/// What the side holding `mine` owes the side holding `theirs`: every memory
/// they have never seen, live or deleted, and the deletion of every memory
/// they still hold live.
pub fn outgoing(mine: &[ManifestEntry], theirs: &[ManifestEntry]) -> Outgoing {
    let theirs: HashMap<&str, EntryState> = theirs
        .iter()
        .map(|(global_id, state)| (global_id.as_str(), *state))
        .collect();
    let mut owed = Outgoing {
        records: Vec::new(),
        tombstones: Vec::new(),
    };
    for (global_id, state) in mine {
        match (theirs.get(global_id.as_str()), state) {
            (None, _) => owed.records.push(global_id.clone()),
            (Some(EntryState::Live), EntryState::Deleted) => {
                owed.tombstones.push(global_id.clone());
            }
            _ => {}
        }
    }
    owed
}

fn invalid(global_id: &str, reason: &str) -> Error {
    Error::Sync(format!("invalid record {global_id:?}: {reason}"))
}

fn check_global_id(global_id: &str) -> Result<()> {
    match uuid::Uuid::parse_str(global_id) {
        Ok(_) => Ok(()),
        Err(_) => Err(invalid(global_id, "its global id is not a UUID")),
    }
}

fn check_timestamp(global_id: &str, field: &str, value: &str) -> Result<()> {
    if parse_timestamp(value).is_ok_and(|stored| stored == value) {
        Ok(())
    } else {
        Err(invalid(
            global_id,
            &format!("{field} {value:?} is not a timestamp in the stored format"),
        ))
    }
}

/// Checks what a peer sent against the rules every local write follows, so
/// that nothing enters the database through sync that a local store or
/// delete could not have written. The error names the first offending record.
pub fn validate(records: &[SyncRecord], tombstones: &[Tombstone]) -> Result<()> {
    for record in records {
        let id = record.global_id.as_str();
        check_global_id(id)?;
        check_timestamp(id, "created_at", &record.created_at)?;
        if let Some(project) = &record.project
            && !matches!(ProjectRef::parse(project), Ok(ProjectRef::Named(name)) if name == *project)
        {
            return Err(invalid(id, &format!("{project:?} is not a project name")));
        }
        match &record.deleted_at {
            Some(deleted_at) => {
                check_timestamp(id, "deleted_at", deleted_at)?;
                if !record.content.is_empty() || !record.tags.is_empty() {
                    return Err(invalid(
                        id,
                        "a deleted memory must not carry content or tags",
                    ));
                }
            }
            None => {
                if record.deleted_by_peer.is_some() {
                    return Err(invalid(id, "only a deleted memory names who deleted it"));
                }
                if !normalize_content(&record.content)
                    .is_ok_and(|content| content == record.content)
                {
                    return Err(invalid(id, "its content is empty or not trimmed"));
                }
                if !normalize_tags(&record.tags).is_ok_and(|tags| tags == record.tags) {
                    return Err(invalid(id, "its tags are not normalized"));
                }
            }
        }
    }
    for tombstone in tombstones {
        check_global_id(&tombstone.global_id)?;
        check_timestamp(&tombstone.global_id, "deleted_at", &tombstone.deleted_at)?;
    }
    Ok(())
}

/// Cuts memories into `records` messages: a message ends after
/// `RECORDS_PER_MESSAGE` memories, or before the memory that would take its
/// content past `CONTENT_BYTES_PER_MESSAGE`.
pub fn batches(records: Vec<SyncRecord>) -> Vec<Vec<SyncRecord>> {
    let mut batches: Vec<Vec<SyncRecord>> = Vec::new();
    let mut content_bytes = 0;
    for record in records {
        let size = record.content.len();
        let full = batches.last().is_none_or(|batch| {
            batch.len() >= RECORDS_PER_MESSAGE || content_bytes + size > CONTENT_BYTES_PER_MESSAGE
        });
        if full {
            batches.push(Vec::new());
            content_bytes = 0;
        }
        content_bytes += size;
        batches
            .last_mut()
            .expect("a batch was started above if there was none")
            .push(record);
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryType;
    use crate::sync::protocol::EntryState::{Deleted, Live};

    const A: &str = "0199a8c0-0000-7000-8000-00000000000a";
    const B: &str = "0199a8c0-0000-7000-8000-00000000000b";
    const C: &str = "0199a8c0-0000-7000-8000-00000000000c";
    const D: &str = "0199a8c0-0000-7000-8000-00000000000d";
    const E: &str = "0199a8c0-0000-7000-8000-00000000000e";
    const F: &str = "0199a8c0-0000-7000-8000-00000000000f";
    const T0: &str = "2026-10-07T10:00:00.000Z";
    const LATER: &str = "2026-10-07T11:00:00.000Z";

    fn entries(list: &[(&str, EntryState)]) -> Vec<ManifestEntry> {
        list.iter()
            .map(|(id, state)| (id.to_string(), *state))
            .collect()
    }

    fn live(global_id: &str) -> SyncRecord {
        SyncRecord {
            global_id: global_id.into(),
            project: Some("recollect".into()),
            memory_type: MemoryType::Note,
            content: "A note".into(),
            tags: vec!["sync".into()],
            origin_peer: None,
            created_at: T0.into(),
            deleted_at: None,
            deleted_by_peer: None,
        }
    }

    fn deleted(global_id: &str) -> SyncRecord {
        SyncRecord {
            content: String::new(),
            tags: Vec::new(),
            deleted_at: Some(LATER.into()),
            deleted_by_peer: Some("SHA256:deleter".into()),
            ..live(global_id)
        }
    }

    fn tombstone(global_id: &str) -> Tombstone {
        Tombstone {
            global_id: global_id.into(),
            deleted_at: LATER.into(),
            deleted_by_peer: None,
        }
    }

    fn message_of(result: Result<()>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn a_manifest_carries_every_row_with_its_state() {
        let rows = vec![(A.to_string(), false), (B.to_string(), true)];
        assert_eq!(manifest(rows), entries(&[(A, Live), (B, Deleted)]));
    }

    #[test]
    fn the_hash_ignores_order_and_changes_with_an_insert_or_a_deletion() {
        let base = manifest_hash(&entries(&[(A, Live), (B, Live)]));
        assert_eq!(base.len(), 64, "a SHA-256 in hex");
        assert_eq!(base, manifest_hash(&entries(&[(B, Live), (A, Live)])));
        assert_ne!(
            base,
            manifest_hash(&entries(&[(A, Live), (B, Live), (C, Live)]))
        );
        assert_ne!(base, manifest_hash(&entries(&[(A, Live), (B, Deleted)])));
        assert_ne!(base, manifest_hash(&[]));
    }

    #[test]
    fn each_side_owes_what_the_other_lacks() {
        let mine = entries(&[
            (A, Live),    // they have never seen it
            (B, Deleted), // they have never seen it either
            (C, Deleted), // they still hold it live
            (D, Live),    // they have deleted it
            (E, Live),    // both hold it
            (F, Deleted), // both have deleted it
        ]);
        let theirs = entries(&[(C, Live), (D, Deleted), (E, Live), (F, Deleted)]);
        let owed = outgoing(&mine, &theirs);
        assert_eq!(owed.records, [A, B], "full rows, live or deleted");
        assert_eq!(owed.tombstones, [C], "a deletion of what they hold live");

        let nothing = outgoing(&theirs, &theirs);
        assert!(nothing.records.is_empty() && nothing.tombstones.is_empty());
    }

    #[test]
    fn well_formed_changes_pass() {
        let global = SyncRecord {
            project: None,
            tags: Vec::new(),
            ..live(B)
        };
        validate(&[live(A), global, deleted(C)], &[tombstone(D)]).unwrap();
        validate(&[], &[]).unwrap();
    }

    #[test]
    fn a_record_that_no_local_store_could_have_written_is_refused_with_the_reason() {
        let cases = [
            (
                SyncRecord {
                    global_id: "x".into(),
                    ..live(A)
                },
                "invalid record \"x\": its global id is not a UUID".to_string(),
            ),
            (
                SyncRecord {
                    created_at: "2026-10-07T10:00:00Z".into(),
                    ..live(A)
                },
                format!(
                    "invalid record {A:?}: created_at \"2026-10-07T10:00:00Z\" is not a timestamp in the stored format"
                ),
            ),
            (
                SyncRecord {
                    project: Some("Has Space".into()),
                    ..live(A)
                },
                format!("invalid record {A:?}: \"Has Space\" is not a project name"),
            ),
            (
                SyncRecord {
                    project: Some("global".into()),
                    ..live(A)
                },
                format!("invalid record {A:?}: \"global\" is not a project name"),
            ),
            (
                SyncRecord {
                    content: " padded ".into(),
                    ..live(A)
                },
                format!("invalid record {A:?}: its content is empty or not trimmed"),
            ),
            (
                SyncRecord {
                    content: String::new(),
                    ..live(A)
                },
                format!("invalid record {A:?}: its content is empty or not trimmed"),
            ),
            (
                SyncRecord {
                    tags: vec!["B".into(), "a".into()],
                    ..live(A)
                },
                format!("invalid record {A:?}: its tags are not normalized"),
            ),
            (
                SyncRecord {
                    deleted_by_peer: Some("SHA256:x".into()),
                    ..live(A)
                },
                format!("invalid record {A:?}: only a deleted memory names who deleted it"),
            ),
            (
                SyncRecord {
                    content: "still here".into(),
                    ..deleted(A)
                },
                format!("invalid record {A:?}: a deleted memory must not carry content or tags"),
            ),
            (
                SyncRecord {
                    deleted_at: Some("yesterday".into()),
                    ..deleted(A)
                },
                format!(
                    "invalid record {A:?}: deleted_at \"yesterday\" is not a timestamp in the stored format"
                ),
            ),
        ];
        for (record, expected) in cases {
            assert_eq!(message_of(validate(&[live(B), record], &[])), expected);
        }
    }

    #[test]
    fn a_malformed_tombstone_is_refused() {
        let no_uuid = Tombstone {
            global_id: "x".into(),
            ..tombstone(A)
        };
        assert_eq!(
            message_of(validate(&[], &[no_uuid])),
            "invalid record \"x\": its global id is not a UUID"
        );
        let no_time = Tombstone {
            deleted_at: "soon".into(),
            ..tombstone(A)
        };
        assert_eq!(
            message_of(validate(&[], &[no_time])),
            format!(
                "invalid record {A:?}: deleted_at \"soon\" is not a timestamp in the stored format"
            )
        );
    }

    #[test]
    fn memories_are_batched_by_count_and_by_size() {
        let sizes = |records: Vec<SyncRecord>| -> Vec<usize> {
            batches(records).iter().map(Vec::len).collect()
        };
        assert_eq!(sizes(Vec::new()), [0usize; 0]);
        assert_eq!(sizes((0..450).map(|_| live(A)).collect()), [200, 200, 50]);

        let megabytes = |count: usize| SyncRecord {
            content: "x".repeat(count * 1024 * 1024),
            ..live(A)
        };
        assert_eq!(
            sizes(vec![
                megabytes(3),
                megabytes(3),
                live(A),
                megabytes(5),
                live(A)
            ]),
            [1, 2, 1, 1],
            "a message ends before it would pass 4 MiB of content; a larger memory travels alone"
        );
    }

    #[test]
    fn the_hash_of_bytes_is_lowercase_hex() {
        assert_eq!(
            crate::sync::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
