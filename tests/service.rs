mod common;

use recollect::Error;
use recollect::config::Config;
use recollect::db::Database;
use recollect::embed::MODEL_ID;
use recollect::filter::Filter;
use recollect::memory::{Embedded, MemoryType, NewRecord, ProjectRef};
use recollect::service::{Context, ProjectCount, Recollect, StoreInput, TagCount};
use tempfile::TempDir;

fn config(dir: &TempDir) -> Config {
    Config::load_from(dir.path().to_path_buf(), Some(common::model_dir())).unwrap()
}

fn app(dir: &TempDir) -> Recollect {
    Recollect::open_with_embedder(config(dir), Box::new(common::shared_model())).unwrap()
}

fn named(name: &str) -> ProjectRef {
    ProjectRef::Named(name.to_string())
}

fn input(content: &str, project: ProjectRef, memory_type: MemoryType) -> StoreInput {
    StoreInput {
        content: content.to_string(),
        project,
        memory_type,
        tags: Vec::new(),
    }
}

fn note(content: &str, project: ProjectRef) -> StoreInput {
    input(content, project, MemoryType::Note)
}

/// A config whose model directory is a regular file, so the model cannot load.
fn config_without_model(dir: &TempDir) -> Config {
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    Config::load_from(dir.path().to_path_buf(), Some(blocker)).unwrap()
}

/// Stores a memory with vectors from another model, as an older binary would have.
fn insert_with_other_model(config: &Config) {
    Database::open(&config.database_path())
        .unwrap()
        .insert_memory(
            &old_record("old", "written with the old model"),
            Some(&Embedded {
                model_id: "all-minilm-l6-v2".into(),
                chunks: vec![vec![0.1; 384]],
            }),
        )
        .unwrap();
}

fn old_record(global_id: &str, content: &str) -> NewRecord {
    NewRecord {
        global_id: global_id.to_string(),
        project: None,
        memory_type: MemoryType::Note,
        content: content.to_string(),
        tags: Vec::new(),
        origin_peer: None,
        created_at: "2026-01-01T00:00:00.000Z".to_string(),
    }
}

#[test]
fn stored_memories_are_embedded_and_found_by_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    let sqlite = app
        .store(note(
            "We keep every memory in a single SQLite file with a project column.",
            named("recollect"),
        ))
        .unwrap();
    app.store(note(
        "Banana bread needs very ripe bananas.",
        named("kitchen"),
    ))
    .unwrap();
    assert_eq!(sqlite.warning, None);

    // No word of the query occurs in either memory: only the vector arm can rank them.
    let outcome = app
        .search("database design decision", &Filter::default(), 5)
        .unwrap();
    assert_eq!(outcome.warning, None);
    assert_eq!(outcome.results[0].memory.id, sqlite.id);

    let status = app.status().unwrap();
    assert_eq!(status.pending_embeddings, 0);
    assert_eq!(status.stored_embedding_model.as_deref(), Some(MODEL_ID));
    assert!(status.vectors_usable);
}

#[test]
fn input_is_normalized_before_storing() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    let stored = app
        .store(StoreInput {
            tags: vec![" B".into(), "a".into(), "b".into()],
            ..note("  padded content \n", ProjectRef::Global)
        })
        .unwrap();
    let memory = app.show(stored.id).unwrap();
    assert_eq!(memory.content, "padded content");
    assert_eq!(memory.tags, ["a", "b"]);
    assert_eq!(memory.project, None);
    assert_eq!(memory.global_id, stored.global_id);
}

#[test]
fn invalid_input_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    assert!(matches!(
        app.store(note(" \n", ProjectRef::Global)),
        Err(Error::EmptyContent)
    ));
    let bad_tag = StoreInput {
        tags: vec!["".into()],
        ..note("x", ProjectRef::Global)
    };
    assert!(matches!(app.store(bad_tag), Err(Error::InvalidTag(_))));
    assert!(app.list(&Filter::default(), 10).unwrap().is_empty());
}

#[test]
fn deleted_memories_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    let id = app
        .store(note("short-lived", ProjectRef::Global))
        .unwrap()
        .id;
    app.delete(id).unwrap();
    assert!(matches!(app.show(id), Err(Error::NotFound(_))));
    assert!(matches!(app.delete(id), Err(Error::NotFound(_))));
    assert!(
        app.search("short-lived", &Filter::default(), 10)
            .unwrap()
            .results
            .is_empty()
    );
}

#[test]
fn context_gathers_the_latest_session_and_recent_notes_and_todos() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    app.store(input("first session", named("p"), MemoryType::Session))
        .unwrap();
    for n in 0..11 {
        app.store(note(&format!("note {n}"), named("p"))).unwrap();
    }
    app.store(input("latest session", named("p"), MemoryType::Session))
        .unwrap();
    app.store(input("open todo", named("p"), MemoryType::Todo))
        .unwrap();
    app.store(input("elsewhere", named("q"), MemoryType::Session))
        .unwrap();

    let Context::Project {
        project,
        last_session,
        recent_notes_todos,
    } = app.context(Some(&named("p"))).unwrap()
    else {
        panic!("a project context was requested");
    };
    assert_eq!(project, "p");
    assert_eq!(last_session.unwrap().content, "latest session");
    assert_eq!(recent_notes_todos.len(), 10);
    assert_eq!(recent_notes_todos[0].content, "open todo");
    assert_eq!(recent_notes_todos[1].content, "note 10");

    let Context::AllProjects {
        project,
        recent_sessions,
        recent_notes_todos,
    } = app.context(None).unwrap()
    else {
        panic!("a cross-project context was requested");
    };
    assert_eq!(project, None);
    let sessions: Vec<&str> = recent_sessions.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(sessions, ["elsewhere", "latest session", "first session"]);
    assert_eq!(recent_notes_todos.len(), 10);

    let Context::Project { project, .. } = app.context(Some(&ProjectRef::Global)).unwrap() else {
        panic!("global is a project context");
    };
    assert_eq!(project, "global");
}

#[test]
fn projects_and_tags_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    app.store(StoreInput {
        tags: vec!["x".into()],
        ..note("a", named("p"))
    })
    .unwrap();
    app.store(StoreInput {
        tags: vec!["x".into(), "y".into()],
        ..note("b", named("p"))
    })
    .unwrap();
    app.store(StoreInput {
        tags: vec!["y".into()],
        ..note("c", ProjectRef::Global)
    })
    .unwrap();
    assert_eq!(
        app.projects().unwrap(),
        [
            ProjectCount {
                name: "global".into(),
                count: 1
            },
            ProjectCount {
                name: "p".into(),
                count: 2
            }
        ]
    );
    let only_p = Filter {
        project: Some(named("p")),
        ..Filter::default()
    };
    assert_eq!(
        app.tags(&only_p, 20).unwrap(),
        [
            TagCount {
                tag: "x".into(),
                count: 2
            },
            TagCount {
                tag: "y".into(),
                count: 1
            }
        ]
    );
}

#[test]
fn vectors_from_another_model_degrade_to_full_text_until_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(&dir);
    insert_with_other_model(&config);
    let mut app = Recollect::open_with_embedder(config, Box::new(common::shared_model())).unwrap();

    let stored = app
        .store(note("written with the new model", ProjectRef::Global))
        .unwrap();
    let warning = stored
        .warning
        .expect("storing under a model mismatch must warn");
    assert!(
        warning.starts_with(&format!(
            "stored #{} without embedding: stored vectors come from all-minilm-l6-v2",
            stored.id
        )),
        "{warning}"
    );

    let search = app.search("written", &Filter::default(), 10).unwrap();
    assert!(
        search
            .warning
            .as_deref()
            .unwrap()
            .starts_with("full-text search only: stored vectors come from")
    );
    assert_eq!(search.results.len(), 2);

    let before = app.status().unwrap();
    assert!(!before.vectors_usable);
    assert_eq!(
        before.vectors_reason.as_deref(),
        Some(
            "stored vectors come from all-minilm-l6-v2, but the model is bge-small-en-v1.5-q; run recollect reindex --all"
        )
    );
    assert!(matches!(
        app.reindex(false),
        Err(Error::ModelMismatch { .. })
    ));
    let refused = app.status().unwrap();
    assert_eq!(
        refused.stored_embedding_model.as_deref(),
        Some("all-minilm-l6-v2")
    );
    assert_eq!(refused.pending_embeddings, before.pending_embeddings);
    assert_eq!(app.reindex(true).unwrap(), 2);
    let status = app.status().unwrap();
    assert_eq!(status.stored_embedding_model.as_deref(), Some(MODEL_ID));
    assert_eq!(status.pending_embeddings, 0);
    assert!(status.vectors_usable);
}

#[test]
fn vectors_from_another_model_are_reported_without_loading_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_without_model(&dir);
    insert_with_other_model(&config);
    let mut app = Recollect::open(config).unwrap();
    let warning = app
        .store(note("new", ProjectRef::Global))
        .unwrap()
        .warning
        .unwrap();
    assert!(
        warning.contains("stored vectors come from all-minilm-l6-v2"),
        "{warning}"
    );
}

#[test]
fn status_reports_why_the_model_failed_to_load() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Recollect::open(config_without_model(&dir)).unwrap();
    app.store(note("x", ProjectRef::Global)).unwrap();
    let reason = app.status().unwrap().vectors_reason.unwrap();
    assert!(
        reason.starts_with("the embedding model is unavailable ("),
        "{reason}"
    );
}

#[test]
fn an_unavailable_model_keeps_store_and_search_working() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Recollect::open(config_without_model(&dir)).unwrap();

    let stored = app
        .store(note("kept without vectors", ProjectRef::Global))
        .unwrap();
    let warning = stored.warning.expect("storing without a model must warn");
    assert!(
        warning.starts_with(&format!(
            "stored #{} without embedding: the embedding model is unavailable",
            stored.id
        )),
        "{warning}"
    );
    assert!(
        warning.ends_with("; run recollect reindex once the model is available"),
        "{warning}"
    );

    let search = app.search("kept", &Filter::default(), 10).unwrap();
    assert_eq!(search.results.len(), 1);
    assert!(
        search
            .warning
            .as_deref()
            .unwrap()
            .starts_with("full-text search only: the embedding model is unavailable")
    );

    assert!(matches!(
        app.reindex(true),
        Err(Error::EmbeddingUnavailable(_))
    ));
    let status = app.status().unwrap();
    assert_eq!(status.pending_embeddings, 1);
    assert!(!status.vectors_usable);
}

#[test]
fn reindex_all_without_a_model_keeps_the_stored_vectors() {
    let dir = tempfile::tempdir().unwrap();
    app(&dir)
        .store(note(
            "embedded while the model was there",
            ProjectRef::Global,
        ))
        .unwrap();
    let mut without_model = Recollect::open(config_without_model(&dir)).unwrap();

    assert!(matches!(
        without_model.reindex(true),
        Err(Error::EmbeddingUnavailable(_))
    ));
    let status = without_model.status().unwrap();
    assert_eq!(status.stored_embedding_model.as_deref(), Some(MODEL_ID));
    assert_eq!(status.pending_embeddings, 0);
}

#[test]
fn reindex_embeds_memories_stored_without_vectors() {
    let dir = tempfile::tempdir().unwrap();
    Database::open(&config(&dir).database_path())
        .unwrap()
        .insert_memory(&old_record("pending", "stored while offline"), None)
        .unwrap();
    let mut app = app(&dir);
    assert_eq!(app.status().unwrap().pending_embeddings, 1);
    assert_eq!(app.reindex(false).unwrap(), 1);
    assert_eq!(app.status().unwrap().pending_embeddings, 0);
    assert_eq!(app.reindex(false).unwrap(), 0);
}

#[test]
fn status_of_a_fresh_database() {
    let dir = tempfile::tempdir().unwrap();
    let status = app(&dir).status().unwrap();
    assert_eq!(status.data_dir, dir.path().display().to_string());
    assert_eq!(
        status.database,
        dir.path().join("memories.db").display().to_string()
    );
    assert_eq!(
        (status.memories, status.projects, status.pending_embeddings),
        (0, 0, 0)
    );
    assert_eq!(status.embedding_model, MODEL_ID);
    assert_eq!(status.stored_embedding_model, None);
    assert!(status.vectors_usable);
    assert_eq!(status.vectors_reason, None);
}
