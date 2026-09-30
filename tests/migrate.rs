mod common;

use std::path::PathBuf;

use common::ruby::{PRE_SYNC, RubyRow, WITH_SOURCE, WITHOUT_SOURCE, insert, ruby_file};
use recollect::Error;
use recollect::memory::{MemoryType, NewRecord};
use recollect::migrate::{Rename, RubyMemories, read_ruby_data};
use tempfile::TempDir;

/// The creation time `RubyRow::note` gives every row.
const CREATED: &str = "2026-03-01T10:00:00.000Z";

fn read(dir: &TempDir) -> recollect::Result<RubyMemories> {
    read_ruby_data(dir.path(), &[])
}

fn rename(raw: &str) -> Rename {
    raw.parse().unwrap()
}

/// The message of a `RubyData` error, after checking that it is about `path`.
fn message_about(path: PathBuf, result: recollect::Result<RubyMemories>) -> String {
    match result {
        Err(Error::RubyData {
            path: actual,
            message,
        }) => {
            assert_eq!(actual, path, "{message}");
            message
        }
        other => panic!("expected an error about {}, got {other:?}", path.display()),
    }
}

fn record(
    global_id: &str,
    project: Option<&str>,
    memory_type: MemoryType,
    content: &str,
) -> NewRecord {
    NewRecord {
        global_id: global_id.to_string(),
        project: project.map(String::from),
        memory_type,
        content: content.to_string(),
        tags: Vec::new(),
        origin_peer: None,
        created_at: CREATED.to_string(),
    }
}

#[test]
fn memories_become_records_of_the_project_their_file_names() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "global.db", WITH_SOURCE),
        &RubyRow::note("g-1", "a global note"),
    );
    let fera = ruby_file(dir.path(), "projects/fera.db", WITHOUT_SOURCE);
    insert(
        &fera,
        &RubyRow {
            memory_type: "session".into(),
            ..RubyRow::note("f-1", "a session")
        },
    );
    insert(
        &fera,
        &RubyRow {
            memory_type: "todo".into(),
            ..RubyRow::note("f-2", "a todo")
        },
    );
    // Every fixture row has an origin_peer; no record carries one.
    assert_eq!(
        read(&dir).unwrap(),
        RubyMemories {
            records: vec![
                record("f-1", Some("fera"), MemoryType::Session, "a session"),
                record("f-2", Some("fera"), MemoryType::Todo, "a todo"),
                record("g-1", None, MemoryType::Note, "a global note"),
            ],
            chunks_skipped: 0,
            tombstones_skipped: 0,
        }
    );
}

#[test]
fn chunk_rows_and_tombstones_are_skipped_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let fera = ruby_file(dir.path(), "projects/fera.db", WITHOUT_SOURCE);
    insert(&fera, &RubyRow::note("f-1", "a long memory"));
    insert(
        &fera,
        &RubyRow {
            memory_type: "_chunk".into(),
            ..RubyRow::note("f-2", "a long")
        },
    );
    // A tombstone is skipped before its (empty) content is checked.
    insert(
        &fera,
        &RubyRow {
            deleted_at: Some(CREATED.into()),
            ..RubyRow::note("f-3", "")
        },
    );
    let memories = read(&dir).unwrap();
    assert_eq!(
        memories.records,
        [record(
            "f-1",
            Some("fera"),
            MemoryType::Note,
            "a long memory"
        )]
    );
    assert_eq!(
        (memories.chunks_skipped, memories.tombstones_skipped),
        (1, 1)
    );
}

#[test]
fn content_tags_and_timestamps_are_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let fera = ruby_file(dir.path(), "projects/fera.db", WITHOUT_SOURCE);
    insert(
        &fera,
        &RubyRow {
            content: "  padded\n".into(),
            tags: Some(r#"[" Sync", "decision", "sync"]"#.into()),
            created_at: Some("2026-03-01T11:00:00+01:00".into()),
            ..RubyRow::note("f-1", "")
        },
    );
    insert(
        &fera,
        &RubyRow {
            tags: None,
            ..RubyRow::note("f-2", "untagged")
        },
    );
    let records = read(&dir).unwrap().records;
    assert_eq!(
        records[0],
        NewRecord {
            tags: vec!["decision".into(), "sync".into()],
            ..record("f-1", Some("fera"), MemoryType::Note, "padded")
        }
    );
    assert_eq!(records[1].tags, Vec::<String>::new());
}

#[test]
fn records_come_oldest_first_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let on_day = |day: u32, global_id: &str, content: &str| RubyRow {
        created_at: Some(format!("2026-03-{day:02}T10:00:00.000Z")),
        ..RubyRow::note(global_id, content)
    };
    insert(
        &ruby_file(dir.path(), "global.db", WITHOUT_SOURCE),
        &on_day(5, "g-1", "newest"),
    );
    insert(
        &ruby_file(dir.path(), "projects/adam.db", WITHOUT_SOURCE),
        &on_day(3, "a-1", "middle"),
    );
    insert(
        &ruby_file(dir.path(), "projects/fera.db", WITHOUT_SOURCE),
        &on_day(1, "f-1", "oldest"),
    );
    let contents: Vec<String> = read(&dir)
        .unwrap()
        .records
        .into_iter()
        .map(|record| record.content)
        .collect();
    assert_eq!(contents, ["oldest", "middle", "newest"]);
}

#[test]
fn files_without_rows_are_skipped_whatever_their_name_or_schema() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "global.db", WITHOUT_SOURCE),
        &RubyRow::note("g-1", "the only memory"),
    );
    ruby_file(dir.path(), "projects/.db", PRE_SYNC);
    ruby_file(
        dir.path(),
        "projects/unrelated.db",
        "CREATE TABLE notes (body TEXT)",
    );
    // Kept open, so its -wal and -shm files exist while the directory is read.
    let _open = ruby_file(dir.path(), "projects/__all__.db", WITHOUT_SOURCE);
    std::fs::write(dir.path().join("projects/readme.txt"), "not a database").unwrap();
    assert_eq!(read(&dir).unwrap().records.len(), 1);
}

#[test]
fn a_directory_without_ruby_data_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("projects")).unwrap();
    assert_eq!(
        message_about(dir.path().to_path_buf(), read(&dir)),
        "no Ruby data: neither global.db nor projects/*.db"
    );
}

#[test]
fn unexpected_values_fail_naming_the_file_and_the_row() {
    let bad = || RubyRow::note("f-2", "bad");
    for (row, expected) in [
        (
            RubyRow {
                memory_type: "x".into(),
                ..bad()
            },
            r#"memory 2: unknown memory type "x""#,
        ),
        (
            RubyRow {
                global_id: None,
                ..bad()
            },
            "memory 2: no global_id",
        ),
        (
            RubyRow {
                content: " \n ".into(),
                ..bad()
            },
            "memory 2: content must not be empty",
        ),
        (
            RubyRow {
                tags: Some(r#"{"a":1}"#.into()),
                ..bad()
            },
            r#"memory 2: tags "{\"a\":1}" are not a JSON array of strings"#,
        ),
        (
            RubyRow {
                tags: Some(r#"["a",2]"#.into()),
                ..bad()
            },
            r#"memory 2: tags "[\"a\",2]" are not a JSON array of strings"#,
        ),
        (
            RubyRow {
                tags: Some(r#"["a"," "]"#.into()),
                ..bad()
            },
            r#"memory 2: invalid tag " ": tags must not be empty"#,
        ),
        (
            RubyRow {
                created_at: Some("yesterday".into()),
                ..bad()
            },
            r#"memory 2: created_at "yesterday" is not an RFC 3339 timestamp"#,
        ),
        (
            RubyRow {
                created_at: None,
                ..bad()
            },
            "memory 2: no created_at",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fera = ruby_file(dir.path(), "projects/fera.db", WITHOUT_SOURCE);
        insert(&fera, &RubyRow::note("f-1", "fine"));
        insert(&fera, &row);
        assert_eq!(
            message_about(dir.path().join("projects/fera.db"), read(&dir)),
            expected
        );
    }
}

#[test]
fn a_file_with_rows_but_without_global_ids_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    ruby_file(dir.path(), "projects/old.db", PRE_SYNC)
        .execute("INSERT INTO memories (content) VALUES ('before sync')", [])
        .unwrap();
    assert_eq!(
        message_about(dir.path().join("projects/old.db"), read(&dir)),
        "the memories table has no global_id column"
    );
}

#[test]
fn a_table_without_deleted_at_holds_no_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    ruby_file(
        dir.path(),
        "global.db",
        "CREATE TABLE memories (id INTEGER PRIMARY KEY, content TEXT, memory_type TEXT, tags TEXT, created_at TEXT, global_id TEXT)",
    )
    .execute(
        "INSERT INTO memories (content, memory_type, tags, created_at, global_id) VALUES ('kept', 'note', '[]', ?1, 'g-1')",
        [CREATED],
    )
    .unwrap();
    assert_eq!(
        read(&dir).unwrap().records,
        [record("g-1", None, MemoryType::Note, "kept")]
    );
}

#[test]
fn a_project_file_with_rows_needs_a_valid_project_name() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "projects/Not Valid.db", WITHOUT_SOURCE),
        &RubyRow::note("n-1", "x"),
    );
    assert_eq!(
        message_about(dir.path().join("projects/Not Valid.db"), read(&dir)),
        r#"invalid project name "Not Valid": allowed are a-z 0-9 . _ -"#
    );
}

#[test]
fn a_file_that_is_not_sqlite_is_named() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("projects")).unwrap();
    std::fs::write(
        dir.path().join("projects/junk.db"),
        "not a database, but long enough to have a header",
    )
    .unwrap();
    assert_eq!(
        message_about(dir.path().join("projects/junk.db"), read(&dir)),
        "file is not a database"
    );
}

#[test]
fn a_global_id_in_two_files_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "projects/a.db", WITHOUT_SOURCE),
        &RubyRow::note("same", "copied"),
    );
    let b = ruby_file(dir.path(), "projects/b.db", WITHOUT_SOURCE);
    insert(&b, &RubyRow::note("b-1", "own"));
    insert(&b, &RubyRow::note("same", "copied"));
    assert_eq!(
        message_about(dir.path().join("projects/b.db"), read(&dir)),
        format!(
            r#"memory 2: global_id "same" is also memory 1 of {}"#,
            dir.path().join("projects/a.db").display()
        )
    );
}

#[test]
fn memories_are_read_while_the_ruby_server_holds_its_files_open() {
    let dir = tempfile::tempdir().unwrap();
    let server = ruby_file(dir.path(), "global.db", WITHOUT_SOURCE);
    insert(
        &server,
        &RubyRow::note("g-1", "committed, not yet checkpointed"),
    );
    let wal = std::fs::metadata(dir.path().join("global.db-wal")).unwrap();
    assert!(
        wal.len() > 0,
        "the row must still sit in the write-ahead log"
    );
    assert_eq!(read(&dir).unwrap().records.len(), 1);
    // The read leaves nothing behind that would stop the server from writing.
    insert(
        &server,
        &RubyRow::note("g-2", "written after the migration read"),
    );
}

#[test]
fn a_rename_splits_at_the_first_equals_sign() {
    assert_eq!(
        "a-b=a_b".parse::<Rename>(),
        Ok(Rename {
            from: "a-b".into(),
            to: "a_b".into()
        })
    );
    assert_eq!(
        "a=b=c".parse::<Rename>(),
        Ok(Rename {
            from: "a".into(),
            to: "b=c".into()
        })
    );
    assert_eq!(
        "a-b".parse::<Rename>(),
        Err(r#"expected FROM=TO, got "a-b""#.to_string())
    );
}

#[test]
fn renames_move_memories_to_another_project_before_names_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    for (file, global_id) in [
        ("projects/guerrilla-redmine.db", "d-1"),
        ("projects/guerrilla_redmine.db", "u-1"),
        ("projects/Old Name.db", "o-1"),
        ("projects/misc.db", "m-1"),
    ] {
        insert(
            &ruby_file(dir.path(), file, WITHOUT_SOURCE),
            &RubyRow::note(global_id, "x"),
        );
    }
    let renames = [
        rename("guerrilla-redmine=guerrilla_redmine"),
        rename("Old Name=old-name"),
        rename("misc=GLOBAL"),
    ];
    let projects: Vec<(String, Option<String>)> = read_ruby_data(dir.path(), &renames)
        .unwrap()
        .records
        .into_iter()
        .map(|record| (record.global_id, record.project))
        .collect();
    assert_eq!(
        projects,
        [
            ("d-1".to_string(), Some("guerrilla_redmine".to_string())),
            ("m-1".to_string(), None),
            ("o-1".to_string(), Some("old-name".to_string())),
            ("u-1".to_string(), Some("guerrilla_redmine".to_string())),
        ]
    );
}

#[test]
fn a_rename_must_name_a_project_file_once() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "projects/misc.db", WITHOUT_SOURCE),
        &RubyRow::note("m-1", "x"),
    );
    let projects = dir.path().join("projects");
    assert_eq!(
        message_about(
            projects.clone(),
            read_ruby_data(dir.path(), &[rename("mics=other")])
        ),
        r#"no project "mics" to rename"#
    );
    assert_eq!(
        message_about(
            projects,
            read_ruby_data(dir.path(), &[rename("misc=a"), rename("misc=b")])
        ),
        r#"project "misc" is renamed twice"#
    );
}

#[test]
fn a_rename_to_an_invalid_project_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(dir.path(), "projects/misc.db", WITHOUT_SOURCE),
        &RubyRow::note("m-1", "x"),
    );
    assert!(matches!(
        read_ruby_data(dir.path(), &[rename("misc=a b")]),
        Err(Error::InvalidProject(name)) if name == "a b"
    ));
}
