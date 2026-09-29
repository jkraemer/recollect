//! Text embeddings for memories and queries.

mod chunker;
mod fastembed_model;

use std::cell::RefCell;

pub use chunker::chunk;
pub use fastembed_model::{FastEmbedder, MODEL_ID};

use crate::error::{Error, Result};
use crate::memory::Embedded;

/// Tokens per chunk: the model's 512-token window minus `[CLS]` and `[SEP]`.
pub const MAX_CHUNK_TOKENS: usize = 510;

/// The seam between callers and the model: implementations may run it
/// in-process or delegate to another process.
pub trait Embedder {
    /// Identifies the model; stored as `meta.embedding_model`.
    fn model_id(&self) -> &str;
    /// Content tokens in `text`, without special tokens and without truncation.
    fn count_tokens(&self, text: &str) -> Result<usize>;
    fn embed_passages(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>>;
}

/// Chunks `content` to the model's window and embeds every chunk.
pub fn embed_memory(embedder: &mut dyn Embedder, content: &str) -> Result<Embedded> {
    let failure: RefCell<Option<Error>> = RefCell::new(None);
    let chunks = {
        let counter: &dyn Embedder = &*embedder;
        chunk(
            content,
            MAX_CHUNK_TOKENS,
            &|text| match counter.count_tokens(text) {
                Ok(tokens) => tokens,
                Err(err) => {
                    let mut first = failure.borrow_mut();
                    if first.is_none() {
                        *first = Some(err);
                    }
                    0
                }
            },
        )
    };
    if let Some(err) = failure.into_inner() {
        return Err(err);
    }
    let vectors = embedder.embed_passages(&chunks)?;
    Ok(Embedded {
        model_id: embedder.model_id().to_string(),
        chunks: vectors,
    })
}
