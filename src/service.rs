//! One method per CLI command, combining storage, embeddings and search.

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::config::Config;
use crate::db::Database;
use crate::embed::{Embedder, FastEmbedder, MODEL_ID, embed_memory};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::memory::{
    Memory, MemoryType, NewRecord, ProjectRef, ScoredMemory, normalize_content, normalize_tags,
};
use crate::search::fts_query::build_fts_query;
use crate::search::{self, SearchRequest};
use crate::time::now_timestamp;

pub struct StoreInput {
    pub content: String,
    pub project: ProjectRef,
    pub memory_type: MemoryType,
    pub tags: Vec<String>,
}

#[derive(Debug)]
pub struct StoreOutcome {
    pub id: i64,
    pub global_id: String,
    /// Set when the memory was stored without embeddings.
    pub warning: Option<String>,
}

#[derive(Debug)]
pub struct SearchOutcome {
    pub results: Vec<ScoredMemory>,
    /// Set when the search ran on full text only.
    pub warning: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Context {
    Project {
        project: String,
        last_session: Option<Memory>,
        recent_notes_todos: Vec<Memory>,
    },
    AllProjects {
        /// Always `None`: serializes as `"project": null`.
        project: Option<String>,
        recent_sessions: Vec<Memory>,
        recent_notes_todos: Vec<Memory>,
    },
}

#[derive(Debug, PartialEq, Serialize)]
pub struct ProjectCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Status {
    pub data_dir: String,
    pub database: String,
    pub memories: usize,
    pub projects: usize,
    pub embedding_model: String,
    pub stored_embedding_model: Option<String>,
    pub vectors_usable: bool,
    /// Why vectors are unusable; `None` when they are usable.
    pub vectors_reason: Option<String>,
    pub pending_embeddings: usize,
}

/// Why vector search or embedding cannot run right now.
enum VectorsUnusable {
    Unavailable(String),
    /// Always an `Error::ModelMismatch`.
    ModelMismatch(Error),
}

impl VectorsUnusable {
    fn describe(&self) -> String {
        match self {
            VectorsUnusable::Unavailable(reason) => {
                format!("the embedding model is unavailable ({reason})")
            }
            VectorsUnusable::ModelMismatch(mismatch) => mismatch.to_string(),
        }
    }

    fn store_warning(&self, id: i64) -> String {
        match self {
            VectorsUnusable::Unavailable(_) => format!(
                "stored #{id} without embedding: {}; run recollect reindex once the model is available",
                self.describe()
            ),
            VectorsUnusable::ModelMismatch(_) => {
                format!("stored #{id} without embedding: {}", self.describe())
            }
        }
    }
}

enum EmbedderSlot {
    NotLoaded,
    Ready(Box<dyn Embedder>),
    Failed(String),
}

pub struct Recollect {
    config: Config,
    db: Database,
    embedder: EmbedderSlot,
    /// Receives progress notices, such as the start of a model download.
    notify: Box<dyn Fn(&str)>,
}

impl Recollect {
    /// Opens the database; the embedding model loads on first use.
    pub fn open(config: Config) -> Result<Self> {
        let db = Database::open(&config.database_path())?;
        Ok(Self {
            config,
            db,
            embedder: EmbedderSlot::NotLoaded,
            notify: Box::new(|_| {}),
        })
    }

    /// Opens the database with an embedder that is already loaded.
    pub fn open_with_embedder(config: Config, embedder: Box<dyn Embedder>) -> Result<Self> {
        let mut app = Self::open(config)?;
        app.embedder = EmbedderSlot::Ready(embedder);
        Ok(app)
    }

    /// Sends progress notices to `notify`; without it they are dropped.
    pub fn with_notices(mut self, notify: impl Fn(&str) + 'static) -> Self {
        self.notify = Box::new(notify);
        self
    }

    /// Stores a memory. Embedding problems never block the write: the memory
    /// is stored without vectors and the outcome carries a warning.
    pub fn store(&mut self, input: StoreInput) -> Result<StoreOutcome> {
        let content = normalize_content(&input.content)?;
        let tags = normalize_tags(&input.tags)?;
        let embedded = self.with_vectors(|embedder| embed_memory(embedder, &content))?;
        let record = NewRecord {
            global_id: Uuid::now_v7().to_string(),
            project: input.project.column_value().map(String::from),
            memory_type: input.memory_type,
            content,
            tags,
            origin_peer: None,
            created_at: now_timestamp(),
        };
        let id = self.db.insert_memory(&record, embedded.as_ref().ok())?;
        Ok(StoreOutcome {
            id,
            global_id: record.global_id,
            warning: embedded.err().map(|why| why.store_warning(id)),
        })
    }

    /// Hybrid search; falls back to full text (with a warning) when vectors are unusable.
    pub fn search(&mut self, query: &str, filter: &Filter, limit: usize) -> Result<SearchOutcome> {
        if build_fts_query(query).is_none() {
            return Err(Error::EmptyQuery);
        }
        let query_vector = self.with_vectors(|embedder| embedder.embed_query(query))?;
        let results = search::search(
            &self.db,
            &SearchRequest {
                query,
                query_vector: query_vector.as_deref().ok(),
                filter,
                limit,
                max_vector_distance: self.config.max_vector_distance,
                recency: self.config.recency,
                now: Utc::now(),
            },
        )?;
        Ok(SearchOutcome {
            results,
            warning: query_vector
                .err()
                .map(|why| format!("full-text search only: {}", why.describe())),
        })
    }

    pub fn list(&self, filter: &Filter, limit: usize) -> Result<Vec<Memory>> {
        self.db.list(filter, limit)
    }

    pub fn show(&self, id: i64) -> Result<Memory> {
        self.db.get(id)
    }

    pub fn delete(&mut self, id: i64) -> Result<()> {
        self.db.delete(id, &now_timestamp())
    }

    /// With a project: its latest session and 10 newest notes/todos. Without:
    /// the 5 newest sessions and 10 newest notes/todos across all projects.
    pub fn context(&self, project: Option<&ProjectRef>) -> Result<Context> {
        let of_types = |project: Option<&ProjectRef>, types: Vec<MemoryType>| Filter {
            project: project.cloned(),
            types,
            ..Filter::default()
        };
        let notes_todos = vec![MemoryType::Note, MemoryType::Todo];
        match project {
            Some(project) => Ok(Context::Project {
                project: project.display_name().to_string(),
                last_session: self
                    .db
                    .list(&of_types(Some(project), vec![MemoryType::Session]), 1)?
                    .into_iter()
                    .next(),
                recent_notes_todos: self.db.list(&of_types(Some(project), notes_todos), 10)?,
            }),
            None => Ok(Context::AllProjects {
                project: None,
                recent_sessions: self
                    .db
                    .list(&of_types(None, vec![MemoryType::Session]), 5)?,
                recent_notes_todos: self.db.list(&of_types(None, notes_todos), 10)?,
            }),
        }
    }

    pub fn projects(&self) -> Result<Vec<ProjectCount>> {
        Ok(self
            .db
            .project_counts()?
            .into_iter()
            .map(|(name, count)| ProjectCount { name, count })
            .collect())
    }

    pub fn tags(&self, filter: &Filter, top: usize) -> Result<Vec<TagCount>> {
        Ok(self
            .db
            .tag_counts(filter, top)?
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect())
    }

    /// Embeds live memories that have no vectors; `all` first discards every
    /// stored vector (needed after a model change). Returns how many memories
    /// were embedded. Without `all`, vectors from another model are refused
    /// before the model loads; an unavailable model fails before anything changes.
    pub fn reindex(&mut self, all: bool) -> Result<usize> {
        if !all && let Some(mismatch) = self.vectors_mismatch()? {
            return Err(mismatch);
        }
        self.embedder().map_err(Error::EmbeddingUnavailable)?;
        if all {
            self.db.clear_embeddings()?;
        }
        let mut embedded_count = 0;
        for (id, content) in self.db.pending_embeddings()? {
            let embedder = self.embedder().map_err(Error::EmbeddingUnavailable)?;
            let embedded = embed_memory(embedder, &content)?;
            match self.db.add_embeddings(id, &embedded) {
                Ok(true) => embedded_count += 1,
                // Embedded or deleted by another process since the pending list was read.
                Ok(false) | Err(Error::NotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(embedded_count)
    }

    /// Counts and vector health; never loads or downloads the model.
    pub fn status(&self) -> Result<Status> {
        let vectors_reason = match (self.vectors_mismatch()?, &self.embedder) {
            (Some(mismatch), _) => Some(mismatch.to_string()),
            (None, EmbedderSlot::Failed(reason)) => {
                Some(VectorsUnusable::Unavailable(reason.clone()).describe())
            }
            (None, EmbedderSlot::NotLoaded) if !FastEmbedder::is_cached(&self.config.model_dir) => Some(
                "the embedding model is not downloaded yet; the next store, search or reindex downloads it"
                    .into(),
            ),
            _ => None,
        };
        Ok(Status {
            data_dir: self.config.data_dir.display().to_string(),
            database: self.config.database_path().display().to_string(),
            memories: self.db.live_count()?,
            projects: self.db.project_counts()?.len(),
            embedding_model: self.model_id().to_string(),
            stored_embedding_model: self.db.stored_embedding_model()?,
            vectors_usable: vectors_reason.is_none(),
            vectors_reason,
            pending_embeddings: self.db.pending_embedding_count()?,
        })
    }

    /// Runs `work` with the embedder, or says why vectors are unusable: an
    /// embedding problem is never an error, callers go on without vectors.
    /// Stored vectors from another model are detected before loading the model.
    fn with_vectors<T>(
        &mut self,
        work: impl FnOnce(&mut dyn Embedder) -> Result<T>,
    ) -> Result<std::result::Result<T, VectorsUnusable>> {
        if let Some(mismatch) = self.vectors_mismatch()? {
            return Ok(Err(VectorsUnusable::ModelMismatch(mismatch)));
        }
        Ok(match self.embedder() {
            Ok(embedder) => {
                work(embedder).map_err(|err| VectorsUnusable::Unavailable(reason_of(err)))
            }
            Err(reason) => Err(VectorsUnusable::Unavailable(reason)),
        })
    }

    /// The model this instance embeds with, known without loading it.
    fn model_id(&self) -> &str {
        match &self.embedder {
            EmbedderSlot::Ready(embedder) => embedder.model_id(),
            EmbedderSlot::NotLoaded | EmbedderSlot::Failed(_) => MODEL_ID,
        }
    }

    /// `Error::ModelMismatch` when the stored vectors come from another model.
    fn vectors_mismatch(&self) -> Result<Option<Error>> {
        let current = self.model_id();
        Ok(self
            .db
            .stored_embedding_model()?
            .filter(|stored| stored != current)
            .map(|stored| Error::ModelMismatch {
                stored,
                current: current.to_string(),
            }))
    }

    /// Loads the model on first use; later calls reuse it or repeat the load failure.
    fn embedder(&mut self) -> std::result::Result<&mut dyn Embedder, String> {
        if matches!(self.embedder, EmbedderSlot::NotLoaded) {
            if !FastEmbedder::is_cached(&self.config.model_dir) {
                (self.notify)(&format!(
                    "downloading embedding model {MODEL_ID} to {}",
                    self.config.model_dir.display()
                ));
            }
            self.embedder = match FastEmbedder::load(&self.config.model_dir) {
                Ok(model) => EmbedderSlot::Ready(Box::new(model)),
                Err(err) => EmbedderSlot::Failed(reason_of(err)),
            };
        }
        match &mut self.embedder {
            EmbedderSlot::Ready(embedder) => Ok(embedder.as_mut()),
            EmbedderSlot::Failed(reason) => Err(reason.clone()),
            EmbedderSlot::NotLoaded => unreachable!("the model was loaded above"),
        }
    }
}

/// The bare reason of an embedding failure, without the error's own prefix.
fn reason_of(err: Error) -> String {
    match err {
        Error::EmbeddingUnavailable(reason) => reason,
        other => other.to_string(),
    }
}
