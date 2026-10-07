//! Helpers shared by the integration tests.
#![allow(dead_code)] // each test binary uses a different subset

pub mod release;
pub mod ruby;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use recollect::Result;
use recollect::embed::{Embedder, FastEmbedder, MODEL_ID};

/// One model cache for every test, so the model downloads once per checkout.
pub fn model_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".model-cache")
}

/// The real embedding model, loaded once per test binary.
pub fn shared_model() -> SharedEmbedder {
    static MODEL: OnceLock<Arc<Mutex<FastEmbedder>>> = OnceLock::new();
    let model = MODEL.get_or_init(|| {
        let model = FastEmbedder::load(&model_dir()).unwrap_or_else(|err| {
            panic!(
                "the embedding model must load for this test (the first run downloads it): {err}"
            )
        });
        Arc::new(Mutex::new(model))
    });
    SharedEmbedder(Arc::clone(model))
}

/// An `Embedder` handle onto the one loaded model.
pub struct SharedEmbedder(Arc<Mutex<FastEmbedder>>);

impl SharedEmbedder {
    fn model(&self) -> MutexGuard<'_, FastEmbedder> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Embedder for SharedEmbedder {
    fn model_id(&self) -> &str {
        MODEL_ID
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.model().count_tokens(text)
    }

    fn embed_passages(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.model().embed_passages(texts)
    }

    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        self.model().embed_query(query)
    }
}
