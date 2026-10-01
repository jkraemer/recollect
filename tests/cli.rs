mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};

use assert_cmd::Command;
use common::ruby::{RubyRow, WITHOUT_SOURCE, insert, ruby_file};
use predicates::prelude::*;
use recollect::db::Database;
use recollect::memory::{MemoryType, NewRecord};
use serde_json::{Value, json};

/// `recollect` against `data_dir`, with the model already downloaded so no
/// download notice appears on stderr.
fn recollect(data_dir: &Path) -> Command {
    let _ = common::shared_model();
    let mut command = Command::cargo_bin("recollect").unwrap();
    command
        .env("RECOLLECT_DATA_DIR", data_dir)
        .env("RECOLLECT_MODEL_DIR", common::model_dir());
    command
}

/// Runs a command that must succeed silently on stderr and parses its stdout as JSON.
fn json_of(command: &mut Command) -> Value {
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn store(data_dir: &Path, args: &[&str]) {
    recollect(data_dir)
        .arg("store")
        .args(args)
        .assert()
        .success()
        .stderr("");
}

fn contents(listed: &Value) -> Vec<String> {
    listed
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn version_flag_prints_the_crate_version() {
    Command::cargo_bin("recollect")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("recollect {}\n", env!("CARGO_PKG_VERSION")))
        .stderr("");
}

#[test]
fn store_from_an_argument_then_show_it() {
    let dir = tempfile::tempdir().unwrap();
    recollect(dir.path())
        .args([
            "store",
            "Single SQLite DB",
            "-p",
            "Recollect",
            "-T",
            "sync,Decision",
        ])
        .assert()
        .success()
        .stdout("stored #1\n")
        .stderr("");
    recollect(dir.path())
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(
                r"^#1 · recollect · note · \d{4}-\d{2}-\d{2} · decision, sync\nSingle SQLite DB\n$",
            )
            .unwrap(),
        )
        .stderr("");
}

#[test]
fn store_reads_multi_line_markdown_from_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let content = "## Decision\nUse `sqlite` with \"quotes\" and 'apostrophes'.\n\n- item $HOME\n";
    recollect(dir.path())
        .arg("store")
        .write_stdin(content)
        .assert()
        .success()
        .stdout("stored #1\n")
        .stderr("");
    let memory = json_of(recollect(dir.path()).args(["show", "1", "--json"]));
    assert_eq!(memory["content"], content.trim());
    assert_eq!(memory["project"], Value::Null);
    assert_eq!(memory["memory_type"], "note");
}

#[test]
fn store_json_reports_both_ids() {
    let dir = tempfile::tempdir().unwrap();
    let stored = json_of(recollect(dir.path()).args(["store", "x", "--json"]));
    assert_eq!(stored["id"], 1);
    assert_eq!(stored["global_id"].as_str().unwrap().len(), 36);
}

#[test]
fn store_rejects_empty_and_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    recollect(dir.path())
        .arg("store")
        .write_stdin("  \n")
        .assert()
        .code(1)
        .stdout("")
        .stderr("error: content must not be empty\n");
    recollect(dir.path())
        .arg("store")
        .write_stdin(vec![0xff_u8, 0xfe])
        .assert()
        .code(1)
        .stderr("error: content is not valid UTF-8\n");
    recollect(dir.path())
        .args(["store", "x", "-p", "a b"])
        .assert()
        .code(1)
        .stderr("error: invalid project name \"a b\": allowed are a-z 0-9 . _ -\n");
    recollect(dir.path())
        .args(["store", "x", "-t", "decision"])
        .assert()
        .code(2);
}

#[test]
fn search_returns_scored_json_and_readable_text() {
    let dir = tempfile::tempdir().unwrap();
    store(
        dir.path(),
        &[
            "We keep every memory in one SQLite file.",
            "-p",
            "recollect",
        ],
    );
    store(
        dir.path(),
        &["Banana bread needs ripe bananas.", "-p", "kitchen"],
    );
    let results = json_of(recollect(dir.path()).args(["search", "sqlite", "file", "--json"]));
    let first = &results[0];
    for key in [
        "id",
        "global_id",
        "project",
        "memory_type",
        "content",
        "tags",
        "created_at",
        "score",
    ] {
        assert!(first.get(key).is_some(), "missing {key}");
    }
    assert_eq!(first["project"], "recollect");
    recollect(dir.path())
        .args(["search", "sqlite"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#1 · recollect · note · "))
        .stderr("");
}

#[test]
fn search_treats_fts_syntax_as_words_and_rejects_empty_queries() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["auth bug in login"]);
    recollect(dir.path())
        .args(["search", r#"auth-bug: "unclosed"#])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth bug in login"))
        .stderr("");
    recollect(dir.path())
        .args(["search", "***"])
        .assert()
        .code(1)
        .stderr("error: search query must contain at least one term\n");
}

#[test]
fn list_filters_by_project_type_tags_and_dates() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["one", "-p", "a", "-T", "x"]);
    store(dir.path(), &["two", "-p", "a", "-t", "todo", "-T", "x,y"]);
    store(dir.path(), &["three", "-t", "session"]);
    let list = |args: &[&str]| {
        contents(&json_of(
            recollect(dir.path()).arg("list").args(args).arg("--json"),
        ))
    };
    assert_eq!(list(&[]), ["three", "two", "one"]);
    assert_eq!(list(&["-p", "a"]), ["two", "one"]);
    assert_eq!(list(&["-p", "global"]), ["three"]);
    assert_eq!(list(&["-t", "todo,session"]), ["three", "two"]);
    assert_eq!(list(&["-T", "y,X"]), ["two"]);
    assert_eq!(
        list(&["--since", "2000-01-01", "--until", "2999-12-31"]).len(),
        3
    );
    assert!(list(&["--until", "2000-01-01"]).is_empty());
    assert_eq!(list(&["-l", "1"]), ["three"]);
    recollect(dir.path())
        .args(["list", "--since", "yesterday"])
        .assert()
        .code(1)
        .stderr("error: invalid date \"yesterday\": use YYYY-MM-DD or an RFC 3339 timestamp\n");
}

#[test]
fn delete_hides_a_memory_and_unknown_ids_fail() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["x"]);
    recollect(dir.path())
        .args(["delete", "1"])
        .assert()
        .success()
        .stdout("deleted #1\n")
        .stderr("");
    recollect(dir.path())
        .args(["show", "1"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("error: memory 1 not found\n");
    recollect(dir.path())
        .args(["delete", "1"])
        .assert()
        .code(1)
        .stderr("error: memory 1 not found\n");
}

#[test]
fn context_shows_the_latest_session_and_recent_notes() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["older session", "-p", "p", "-t", "session"]);
    store(dir.path(), &["latest session", "-p", "p", "-t", "session"]);
    store(dir.path(), &["a note", "-p", "p"]);
    store(dir.path(), &["a todo", "-p", "p", "-t", "todo"]);
    recollect(dir.path())
        .args(["context", "-p", "p"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Last session\n#2 · p · session · ",
        ))
        .stderr("");
    let context = json_of(recollect(dir.path()).args(["context", "-p", "p", "--json"]));
    assert_eq!(context["project"], "p");
    assert_eq!(context["last_session"]["content"], "latest session");
    assert_eq!(
        contents(&context["recent_notes_todos"]),
        ["a todo", "a note"]
    );
    let everywhere = json_of(recollect(dir.path()).args(["context", "--json"]));
    assert_eq!(everywhere["project"], Value::Null);
    assert_eq!(
        contents(&everywhere["recent_sessions"]),
        ["latest session", "older session"]
    );
}

#[test]
fn projects_and_tags_count_live_memories() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["a", "-p", "p", "-T", "x"]);
    store(dir.path(), &["b", "-p", "p", "-T", "x,y"]);
    store(dir.path(), &["c", "-T", "y"]);
    recollect(dir.path())
        .arg("projects")
        .assert()
        .success()
        .stdout("global  1\np       2\n")
        .stderr("");
    assert_eq!(
        json_of(recollect(dir.path()).args(["projects", "--json"])),
        json!([{"name": "global", "count": 1}, {"name": "p", "count": 2}])
    );
    recollect(dir.path())
        .arg("tags")
        .assert()
        .success()
        .stdout("x  2\ny  2\n")
        .stderr("");
    assert_eq!(
        json_of(recollect(dir.path()).args(["tags", "-p", "p", "-n", "1", "--json"])),
        json!([{"tag": "x", "count": 2}])
    );
}

#[test]
fn status_and_reindex_report_vector_health() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["x"]);
    let status = json_of(recollect(dir.path()).args(["status", "--json"]));
    assert_eq!(status["memories"], 1);
    assert_eq!(status["pending_embeddings"], 0);
    assert_eq!(status["vectors_usable"], true);
    assert_eq!(status["embedding_model"], "bge-small-en-v1.5-q");
    assert_eq!(
        status["database"],
        dir.path().join("memories.db").display().to_string()
    );
    recollect(dir.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("pending embeddings:  0"))
        .stderr("");
    recollect(dir.path())
        .arg("reindex")
        .assert()
        .success()
        .stdout("embedded 0 memories\n")
        .stderr("");
    recollect(dir.path())
        .args(["reindex", "--all"])
        .assert()
        .success()
        .stdout("embedded 1 memories\n")
        .stderr("");
}

#[test]
fn a_fresh_data_directory_answers_read_commands_with_empty_output() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("fresh");
    for args in [
        &["list"][..],
        &["projects"],
        &["tags"],
        &["search", "anything"],
    ] {
        recollect(&data)
            .args(args)
            .assert()
            .success()
            .stdout("")
            .stderr("");
    }
    recollect(&data)
        .arg("context")
        .assert()
        .success()
        .stdout("Recent sessions\n(none)\n\nRecent notes and todos\n(none)\n")
        .stderr("");
    assert!(data.join("memories.db").is_file());
}

#[test]
fn without_recollect_data_dir_the_home_directory_is_used() {
    let home = tempfile::tempdir().unwrap();
    let _ = common::shared_model();
    Command::cargo_bin("recollect")
        .unwrap()
        .env_remove("RECOLLECT_DATA_DIR")
        .env("HOME", home.path())
        .env("RECOLLECT_MODEL_DIR", common::model_dir())
        .args(["store", "x"])
        .assert()
        .success()
        .stdout("stored #1\n");
    assert!(home.path().join(".recollect").join("memories.db").is_file());
}

#[test]
fn without_a_data_directory_or_home_the_command_fails_with_advice() {
    Command::cargo_bin("recollect")
        .unwrap()
        .env_remove("RECOLLECT_DATA_DIR")
        .env_remove("HOME")
        .env("RECOLLECT_MODEL_DIR", common::model_dir())
        .arg("list")
        .assert()
        .code(1)
        .stdout("")
        .stderr("error: HOME is not set; set RECOLLECT_DATA_DIR to choose the data directory\n");
}

#[test]
fn file_errors_name_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, "").unwrap();
    let data = file.join("data");
    recollect(&data)
        .arg("list")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(format!(
            "error: {}: ",
            data.join("config.toml").display()
        )));
}

#[test]
fn concurrent_writers_all_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let _ = common::shared_model();
    let children: Vec<_> = (0..4)
        .map(|n| {
            StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
                .env("RECOLLECT_DATA_DIR", dir.path())
                .env("RECOLLECT_MODEL_DIR", common::model_dir())
                .args(["store", &format!("concurrent memory {n}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
    }
    assert_eq!(
        json_of(recollect(dir.path()).args(["list", "--json"]))
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn an_unavailable_model_degrades_with_warnings_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    let without_model = || {
        let mut command = recollect(dir.path());
        command.env("RECOLLECT_MODEL_DIR", &blocker);
        command
    };
    without_model()
        .args(["store", "x"])
        .assert()
        .success()
        .stdout("stored #1\n")
        .stderr(predicate::str::starts_with(format!(
            "downloading embedding model bge-small-en-v1.5-q to {}\n",
            blocker.display()
        )))
        .stderr(predicate::str::contains(
            "warning: stored #1 without embedding:",
        ))
        .stderr(predicate::str::contains("error:").not());
    without_model()
        .args(["search", "x"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#1 · global · note · "))
        .stderr(predicate::str::contains("warning: full-text search only:"))
        .stderr(predicate::str::contains("error:").not());
    without_model()
        .arg("reindex")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("error: embedding unavailable:"));
}

/// Stores a memory without going through the CLI, so it has no vectors.
fn insert_without_vectors(data_dir: &Path, global_id: &str, content: &str) {
    Database::open(&data_dir.join("memories.db"))
        .unwrap()
        .insert_memory(
            &NewRecord {
                global_id: global_id.to_string(),
                project: None,
                memory_type: MemoryType::Note,
                content: content.to_string(),
                tags: Vec::new(),
                origin_peer: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            None,
        )
        .unwrap();
}

#[test]
fn concurrent_reindex_runs_embed_each_memory_once() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..20 {
        insert_without_vectors(dir.path(), &format!("m{n}"), &format!("memory number {n}"));
    }
    let _ = common::shared_model();
    let runs: Vec<_> = (0..2)
        .map(|_| {
            StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
                .env("RECOLLECT_DATA_DIR", dir.path())
                .env("RECOLLECT_MODEL_DIR", common::model_dir())
                .arg("reindex")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut embedded = 0;
    for run in runs {
        let output = run.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(stderr, "");
        let stdout = String::from_utf8_lossy(&output.stdout);
        embedded += stdout
            .trim_start_matches("embedded ")
            .trim_end_matches(" memories\n")
            .parse::<usize>()
            .unwrap();
    }
    assert_eq!(embedded, 20);
    let status = json_of(recollect(dir.path()).args(["status", "--json"]));
    assert_eq!(status["pending_embeddings"], 0);
}

#[test]
fn a_reader_that_stops_early_ends_the_command_quietly() {
    let dir = tempfile::tempdir().unwrap();
    // Far more output than a pipe buffers, so the command is still writing when the reader leaves.
    insert_without_vectors(dir.path(), "large", &"x".repeat(300_000));
    let mut child = StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
        .env("RECOLLECT_DATA_DIR", dir.path())
        .env("RECOLLECT_MODEL_DIR", common::model_dir())
        .arg("list")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdout
        .take()
        .unwrap()
        .read_exact(&mut [0_u8; 16])
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

/// A pipe whose reader is already gone: every write to it fails with a broken pipe.
fn pipe_without_reader() -> std::io::PipeWriter {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer
}

#[test]
fn a_stderr_nobody_reads_does_not_stop_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    let run = |args: &[&str]| {
        StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
            .env("RECOLLECT_DATA_DIR", dir.path())
            .env("RECOLLECT_MODEL_DIR", &blocker)
            .args(args)
            .stderr(pipe_without_reader())
            .output()
            .unwrap()
    };
    // Writes the download notice and a warning to stderr.
    let stored = run(&["store", "x"]);
    assert_eq!(stored.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&stored.stdout), "stored #1\n");
    // Writes an error to stderr.
    assert_eq!(run(&["show", "2"]).status.code(), Some(1));
}

#[cfg(target_os = "linux")]
#[test]
fn an_output_write_failure_is_reported_as_an_error() {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), &["x"]);
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let output = StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
        .env("RECOLLECT_DATA_DIR", dir.path())
        .env("RECOLLECT_MODEL_DIR", common::model_dir())
        .args(["show", "1"])
        .stdout(full)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: No space left on device (os error 28)\n"
    );
}

/// A Ruby data directory: a global note, and two spellings of one project,
/// one of which also holds a chunk row.
fn ruby_fixture(dir: &Path) {
    insert(
        &ruby_file(dir, "global.db", WITHOUT_SOURCE),
        &RubyRow::note("g-1", "A global note about tooling."),
    );
    let dashed = ruby_file(dir, "projects/my-proj.db", WITHOUT_SOURCE);
    insert(
        &dashed,
        &RubyRow::note("d-1", "Deploys go through staging."),
    );
    insert(
        &dashed,
        &RubyRow {
            memory_type: "_chunk".into(),
            ..RubyRow::note("d-2", "Deploys go")
        },
    );
    insert(
        &ruby_file(dir, "projects/my_proj.db", WITHOUT_SOURCE),
        &RubyRow::note("u-1", "The staging server runs Debian."),
    );
}

/// `recollect migrate-from-ruby` of `ruby_dir` into `data_dir`, merging the
/// fixture's two spellings of the project.
fn migrate(data_dir: &Path, ruby_dir: &Path) -> Command {
    let mut command = recollect(data_dir);
    command
        .arg("migrate-from-ruby")
        .arg(ruby_dir)
        .args(["--rename", "my-proj=my_proj"]);
    command
}

const FIRST_MIGRATION: &str = "imported 3 memories (0 already present, 1 chunk rows skipped)\ndeleted 0 memories (0 Ruby tombstones)\nembedded 3 memories\n";
const REPEATED_MIGRATION: &str = "imported 0 memories (3 already present, 1 chunk rows skipped)\ndeleted 0 memories (0 Ruby tombstones)\nembedded 0 memories\n";

#[test]
fn migrate_from_ruby_imports_and_embeds_once() {
    let ruby = tempfile::tempdir().unwrap();
    ruby_fixture(ruby.path());
    let data = tempfile::tempdir().unwrap();
    migrate(data.path(), ruby.path())
        .assert()
        .success()
        .stdout(FIRST_MIGRATION)
        .stderr("embedding 3 memories\n");
    assert_eq!(
        json_of(recollect(data.path()).args(["projects", "--json"])),
        json!([{"name": "global", "count": 1}, {"name": "my_proj", "count": 2}])
    );
    assert_eq!(
        json_of(recollect(data.path()).args(["status", "--json"]))["pending_embeddings"],
        0
    );
    migrate(data.path(), ruby.path())
        .assert()
        .success()
        .stdout(REPEATED_MIGRATION)
        .stderr("");
}

#[test]
fn migrate_from_ruby_can_write_into_the_ruby_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    ruby_fixture(dir.path());
    migrate(dir.path(), dir.path())
        .assert()
        .success()
        .stdout(FIRST_MIGRATION)
        .stderr("embedding 3 memories\n");
    migrate(dir.path(), dir.path())
        .assert()
        .success()
        .stdout(REPEATED_MIGRATION)
        .stderr("");
}

#[test]
fn migrate_from_ruby_deletes_what_ruby_deleted_since_the_last_run() {
    let ruby = tempfile::tempdir().unwrap();
    ruby_fixture(ruby.path());
    let data = tempfile::tempdir().unwrap();
    migrate(data.path(), ruby.path()).assert().success();
    rusqlite::Connection::open(ruby.path().join("projects/my-proj.db"))
        .unwrap()
        .execute(
            "UPDATE memories SET deleted_at = '2026-03-05T10:00:00.000Z' WHERE global_id = 'd-1'",
            [],
        )
        .unwrap();
    migrate(data.path(), ruby.path())
        .assert()
        .success()
        .stdout("imported 0 memories (2 already present, 1 chunk rows skipped)\ndeleted 1 memories (1 Ruby tombstones)\nembedded 0 memories\n")
        .stderr("");
    assert_eq!(
        json_of(recollect(data.path()).args(["projects", "--json"])),
        json!([{"name": "global", "count": 1}, {"name": "my_proj", "count": 1}])
    );
    migrate(data.path(), ruby.path())
        .assert()
        .success()
        .stdout("imported 0 memories (2 already present, 1 chunk rows skipped)\ndeleted 0 memories (1 Ruby tombstones)\nembedded 0 memories\n")
        .stderr("");
}

#[test]
fn migrate_from_ruby_without_a_model_warns_and_leaves_memories_pending() {
    let ruby = tempfile::tempdir().unwrap();
    ruby_fixture(ruby.path());
    let data = tempfile::tempdir().unwrap();
    let blocker = data.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    migrate(data.path(), ruby.path())
        .env("RECOLLECT_MODEL_DIR", &blocker)
        .assert()
        .success()
        .stdout("imported 3 memories (0 already present, 1 chunk rows skipped)\ndeleted 0 memories (0 Ruby tombstones)\nembedded 0 memories\n")
        .stderr(predicate::str::contains(
            "warning: stored 3 memories without embedding: the embedding model is unavailable (",
        ))
        .stderr(predicate::str::contains(
            "run recollect reindex once the model is available",
        ))
        .stderr(predicate::str::contains("error:").not());
    assert_eq!(
        json_of(recollect(data.path()).args(["status", "--json"]))["pending_embeddings"],
        3
    );
}

#[test]
fn migrate_from_ruby_refuses_bad_data_and_bad_usage_without_writing() {
    let ruby = tempfile::tempdir().unwrap();
    insert(
        &ruby_file(ruby.path(), "projects/fera.db", WITHOUT_SOURCE),
        &RubyRow {
            memory_type: "x".into(),
            ..RubyRow::note("f-1", "odd")
        },
    );
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    recollect(&data)
        .arg("migrate-from-ruby")
        .arg(ruby.path())
        .assert()
        .code(1)
        .stdout("")
        .stderr(format!(
            "error: {}: memory 1: unknown memory type \"x\"\n",
            ruby.path().join("projects/fera.db").display()
        ));
    recollect(&data)
        .arg("migrate-from-ruby")
        .arg(ruby.path())
        .args(["--rename", "my-proj"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains(
            r#"expected FROM=TO, got "my-proj""#,
        ));
    assert!(!data.exists(), "nothing may be written");
}

/// A directory that project detection sees as a git repository root.
fn fake_repository(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    root
}

/// `recollect hook <event>` with `input` as the hook's JSON on stdin.
fn hook(data_dir: &Path, event: &str, input: &Value) -> Command {
    let mut command = recollect(data_dir);
    command.args(["hook", event]).write_stdin(input.to_string());
    command
}

#[test]
fn hook_session_start_prints_the_memory_of_the_session_directory_project() {
    let data = tempfile::tempdir().unwrap();
    let code = tempfile::tempdir().unwrap();
    let repo = fake_repository(code.path(), "fera");
    store(
        data.path(),
        &[
            "Session: billing\nInvoices are split.",
            "-p",
            "fera",
            "-t",
            "session",
        ],
    );
    store(
        data.path(),
        &[
            "Invoices are immutable once sent.",
            "-p",
            "fera",
            "-T",
            "decision",
        ],
    );
    store(data.path(), &["Unrelated note", "-p", "other"]);
    let input = json!({
        "session_id": "abc123",
        "transcript_path": "/home/u/.claude/projects/fera/abc123.jsonl",
        "cwd": repo.join("src"),
        "hook_event_name": "SessionStart",
        "source": "startup",
        "model": "claude-opus-5-5",
    });
    let output = hook(data.path(), "session-start", &input).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(stderr, "");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with(&format!(
            "# Recollect memory: project fera\n\nProject from the repository directory {}.\n",
            repo.display()
        )),
        "{stdout}"
    );
    assert!(stdout.contains("\n\n## Last session · #1 · "), "{stdout}");
    assert!(
        stdout.contains(
            "\nSession: billing\nInvoices are split.\n\n## Recent notes and todos\n- #2 · note · "
        ),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(" · decision · Invoices are immutable once sent.\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("Unrelated note"), "{stdout}");
}

#[test]
fn hook_session_start_outside_a_repository_shows_recent_memories_everywhere() {
    let data = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    store(data.path(), &["A note in p", "-p", "p"]);
    hook(data.path(), "session-start", &json!({"cwd": elsewhere.path()}))
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "# Recollect memory: no project\n\n{} is not in a git repository and has no .recollect-project file.\n",
            elsewhere.path().display()
        )))
        .stdout(predicate::str::contains(
            "\n\n## Recent notes and todos\n- #1 · p · note · ",
        ))
        .stdout(predicate::str::ends_with(" · A note in p\n"))
        .stderr("");
}

#[test]
fn the_first_session_on_a_machine_creates_the_database_without_loading_the_model() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("fresh");
    let blocker = parent.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    let repo = fake_repository(parent.path(), "fera");
    // Loading the model from a file instead of a directory would print a download notice.
    hook(&data, "session-start", &json!({"cwd": repo}))
        .env("RECOLLECT_MODEL_DIR", &blocker)
        .assert()
        .success()
        .stdout(predicate::str::ends_with(
            "with the content on stdin.\n\nNo memories stored for this project yet.\n",
        ))
        .stderr("");
    assert!(data.join("memories.db").is_file());
}

#[test]
fn hook_input_without_cwd_uses_the_working_directory() {
    let data = tempfile::tempdir().unwrap();
    let code = tempfile::tempdir().unwrap();
    let repo = fake_repository(code.path(), "fera");
    hook(data.path(), "session-start", &json!({}))
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "# Recollect memory: project fera\n",
        ))
        .stderr("");
}

#[test]
fn malformed_hook_input_fails_with_an_error_line() {
    let data = tempfile::tempdir().unwrap();
    for event in ["session-start", "post-compact"] {
        recollect(data.path())
            .args(["hook", event])
            .write_stdin("not json")
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::is_match(r"^error: invalid hook input: [^\n]+\n$").unwrap());
    }
}

#[test]
fn hook_post_compact_stores_the_summary_as_a_session_of_the_project() {
    let data = tempfile::tempdir().unwrap();
    let code = tempfile::tempdir().unwrap();
    let repo = fake_repository(code.path(), "fera");
    let summary = "## Summary\nWe chose `sqlite` with \"quotes\", 'apostrophes' and $HOME.\n\n- next: wire the hook\n";
    hook(
        data.path(),
        "post-compact",
        &json!({
            "session_id": "abc123",
            "transcript_path": "/home/u/.claude/projects/fera/abc123.jsonl",
            "cwd": repo,
            "hook_event_name": "PostCompact",
            "trigger": "auto",
            "compact_summary": summary,
        }),
    )
    .assert()
    .success()
    .stdout("")
    .stderr("");
    let stored = json_of(recollect(data.path()).args(["list", "-p", "fera", "--json"]));
    assert_eq!(stored.as_array().unwrap().len(), 1);
    assert_eq!(stored[0]["memory_type"], "session");
    assert_eq!(stored[0]["tags"], json!(["compaction"]));
    assert_eq!(stored[0]["content"], summary.trim());
    assert_eq!(
        json_of(recollect(data.path()).args(["status", "--json"]))["pending_embeddings"],
        0
    );
    hook(data.path(), "session-start", &json!({"cwd": repo}))
        .assert()
        .success()
        .stdout(predicate::str::contains("\n\n## Last session · #1 · "))
        .stdout(predicate::str::contains(
            " · compaction\n## Summary\nWe chose `sqlite`",
        ))
        .stderr("");
}

#[test]
fn hook_post_compact_outside_a_repository_stores_a_global_session() {
    let data = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    hook(
        data.path(),
        "post-compact",
        &json!({"cwd": elsewhere.path(), "trigger": "manual", "compact_summary": "Summary of a scratch session."}),
    )
    .assert()
    .success()
    .stdout("")
    .stderr("");
    let stored = json_of(recollect(data.path()).args(["list", "--json"]));
    assert_eq!(stored[0]["project"], Value::Null);
    assert_eq!(stored[0]["memory_type"], "session");
    assert_eq!(stored[0]["content"], "Summary of a scratch session.");
}

#[test]
fn hook_post_compact_without_a_summary_stores_nothing() {
    let data = tempfile::tempdir().unwrap();
    for input in [
        json!({"cwd": data.path()}),
        json!({"cwd": data.path(), "compact_summary": null}),
        json!({"cwd": data.path(), "compact_summary": " \n "}),
    ] {
        hook(data.path(), "post-compact", &input)
            .assert()
            .success()
            .stdout("")
            .stderr("");
    }
    assert_eq!(
        json_of(recollect(data.path()).args(["list", "--json"])),
        json!([])
    );
}

#[test]
fn hook_post_compact_without_a_model_stores_the_summary_pending_with_a_warning() {
    let data = tempfile::tempdir().unwrap();
    let blocker = data.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    hook(
        data.path(),
        "post-compact",
        &json!({"cwd": data.path(), "compact_summary": "Summary while the model is unavailable."}),
    )
    .env("RECOLLECT_MODEL_DIR", &blocker)
    .assert()
    .success()
    .stdout("")
    .stderr(predicate::str::contains(
        "warning: stored #1 without embedding:",
    ))
    .stderr(predicate::str::contains("error:").not());
    assert_eq!(
        json_of(recollect(data.path()).args(["status", "--json"]))["pending_embeddings"],
        1
    );
}
