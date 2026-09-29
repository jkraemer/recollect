//! Text embeddings for memories and queries.

mod chunker;

pub use chunker::chunk;

/// Tokens per chunk: the model's 512-token window minus `[CLS]` and `[SEP]`.
pub const MAX_CHUNK_TOKENS: usize = 510;
