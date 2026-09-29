mod common;

use recollect::Error;
use recollect::embed::{Embedder, FastEmbedder, MAX_CHUNK_TOKENS, MODEL_ID, chunk, embed_memory};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}

#[test]
fn passages_embed_to_384_deterministic_dimensions() {
    let mut model = common::shared_model();
    let texts = vec!["first passage".to_string(), "second passage".to_string()];
    let vectors = model.embed_passages(&texts).unwrap();
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|v| v.len() == 384));
    assert_eq!(model.embed_passages(&texts).unwrap(), vectors);
    assert_eq!(model.model_id(), MODEL_ID);
}

#[test]
fn queries_get_the_retrieval_prefix() {
    let mut model = common::shared_model();
    let query = model.embed_query("sqlite schema").unwrap();
    let passage = model
        .embed_passages(&["sqlite schema".to_string()])
        .unwrap()
        .remove(0);
    assert_ne!(query, passage);
}

#[test]
fn related_text_is_closer_than_unrelated_text() {
    let mut model = common::shared_model();
    let query = model
        .embed_query("how do we migrate the database schema")
        .unwrap();
    let passages = vec![
        "Schema migrations run in one transaction tracked by user_version.".to_string(),
        "Banana bread needs very ripe bananas and brown sugar.".to_string(),
    ];
    let vectors = model.embed_passages(&passages).unwrap();
    assert!(cosine(&query, &vectors[0]) > cosine(&query, &vectors[1]));
}

#[test]
fn token_counts_ignore_the_model_window() {
    let model = common::shared_model();
    assert_eq!(model.count_tokens(&"memory ".repeat(1000)).unwrap(), 1000);
}

#[test]
fn token_counts_add_up_across_whitespace() {
    // The chunker packs pieces by summing their counts; this pins that the
    // real tokenizer never merges tokens across whitespace.
    let model = common::shared_model();
    let pieces = [
        "fn main() {",
        "println!(\"hi\");",
        "}",
        "Déjà vu: user_version=3",
        "e-mail@example.com",
    ];
    let joined = pieces.join(" ");
    let sum: usize = pieces.iter().map(|p| model.count_tokens(p).unwrap()).sum();
    assert_eq!(model.count_tokens(&joined).unwrap(), sum);
}

#[test]
fn long_memories_are_chunked_within_the_window() {
    let mut model = common::shared_model();
    let paragraph = "Recollect stores memories in SQLite and searches them by meaning. ".repeat(40);
    let content = format!("# One\n{paragraph}\n\n# Two\n{paragraph}\n\n# Three\n{paragraph}");
    let chunks = chunk(&content, MAX_CHUNK_TOKENS, &|text| {
        model.count_tokens(text).unwrap()
    });
    assert!(chunks.len() >= 3, "{} chunks", chunks.len());
    for piece in &chunks {
        assert!(model.count_tokens(piece).unwrap() <= MAX_CHUNK_TOKENS);
    }
    let embedded = embed_memory(&mut model, &content).unwrap();
    assert_eq!(embedded.model_id, MODEL_ID);
    assert_eq!(embedded.chunks.len(), chunks.len());
    assert!(embedded.chunks.iter().all(|v| v.len() == 384));
}

#[test]
fn memories_with_many_chunks_embed_one_vector_per_chunk_in_order() {
    let mut model = common::shared_model();
    let topics = [
        "sqlite schema migrations",
        "banana bread baking",
        "sailing across the atlantic",
        "quarterly tax filing",
    ];
    let content = topics
        .iter()
        .map(|topic| format!("# {topic}\n{}", format!("Notes about {topic}. ").repeat(90)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let chunks = chunk(&content, MAX_CHUNK_TOKENS, &|text| {
        model.count_tokens(text).unwrap()
    });
    assert!(chunks.len() > 4, "{} chunks", chunks.len());

    let embedded = embed_memory(&mut model, &content).unwrap();
    assert_eq!(embedded.chunks.len(), chunks.len());
    assert!(embedded.chunks.iter().all(|v| v.len() == 384));
    for (index, piece) in chunks.iter().enumerate() {
        let alone = model
            .embed_passages(std::slice::from_ref(piece))
            .unwrap()
            .remove(0);
        let similarities: Vec<f32> = embedded
            .chunks
            .iter()
            .map(|vector| cosine(vector, &alone))
            .collect();
        let best = similarities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(best, index, "chunk {index}: {similarities:?}");
        assert!(
            similarities[index] > 0.999,
            "chunk {index}: {similarities:?}"
        );
    }
}

#[test]
fn an_unusable_model_directory_is_reported_as_unavailable() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let err = FastEmbedder::load(file.path())
        .err()
        .expect("a file cannot hold the model cache");
    assert!(matches!(err, Error::EmbeddingUnavailable(_)), "{err}");
    assert!(!FastEmbedder::is_cached(file.path()));
}
