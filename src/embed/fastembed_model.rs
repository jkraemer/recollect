//! The in-process model: BGE small English v1.5, quantized, run by fastembed.

use std::path::{Path, PathBuf};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use tokenizers::Tokenizer;

use super::Embedder;
use crate::error::{Error, Result};

/// Stored as `meta.embedding_model` for vectors from this model.
pub const MODEL_ID: &str = "bge-small-en-v1.5-q";

/// BGE's retrieval instruction, prepended to queries but not to stored passages.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// The model's hf-hub cache entry inside the model directory.
const MODEL_CACHE_ENTRY: &str = "models--Qdrant--bge-small-en-v1.5-onnx-Q";

pub struct FastEmbedder {
    model: TextEmbedding,
    /// The model's tokenizer without truncation or padding, for counting.
    counter: Tokenizer,
}

impl FastEmbedder {
    /// Loads the model from `model_dir`, downloading it there first if missing.
    pub fn load(model_dir: &Path) -> Result<Self> {
        let options = TextInitOptions::new(EmbeddingModel::BGESmallENV15Q)
            .with_cache_dir(model_dir.to_path_buf())
            .with_show_download_progress(false);
        let model = TextEmbedding::try_new(options).map_err(unavailable)?;
        let mut counter = model.tokenizer.clone();
        counter.with_truncation(None).map_err(unavailable)?;
        counter.with_padding(None);
        Ok(Self { model, counter })
    }

    /// Whether `load` will find the model without downloading it.
    pub fn is_cached(model_dir: &Path) -> bool {
        cache_root(model_dir).join(MODEL_CACHE_ENTRY).is_dir()
    }
}

/// Where fastembed actually caches: it lets `HF_HOME` override the directory
/// it is given (`fastembed::common::pull_from_hf`).
fn cache_root(model_dir: &Path) -> PathBuf {
    std::env::var("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| model_dir.to_path_buf())
}

impl Embedder for FastEmbedder {
    fn model_id(&self) -> &str {
        MODEL_ID
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.counter
            .encode(text, false)
            .map(|encoding| encoding.len())
            .map_err(unavailable)
    }

    fn embed_passages(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.model.embed(texts, None).map_err(unavailable)
    }

    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let mut vectors = self
            .model
            .embed([format!("{QUERY_PREFIX}{query}")], None)
            .map_err(unavailable)?;
        vectors
            .pop()
            .ok_or_else(|| Error::EmbeddingUnavailable("the model returned no vector".to_string()))
    }
}

fn unavailable(err: impl std::fmt::Display) -> Error {
    Error::EmbeddingUnavailable(err.to_string())
}
