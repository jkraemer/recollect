# frozen_string_literal: true

require "test_helper"

module Recollect
  class DatabaseVectorTest < TestCase
    def setup
      super
      @db_path = File.join(TEST_DATA_DIR, "vector_test.db")
    end

    def teardown
      @db&.close
      super
    end

    def test_vectors_disabled_by_default
      @db = Database.new(@db_path)

      refute_predicate @db, :vectors_enabled?
    end

    # Deliberately gated on the File.exist? prediction: an extension that is
    # on disk but fails to load must fail here, not skip.
    def test_load_vector_extension_succeeds
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)

      assert_predicate @db, :vectors_enabled?
    end

    def test_store_embedding_noop_when_vectors_disabled
      @db = Database.new(@db_path)
      memory_id = @db.store(content: "test", memory_type: "note", tags: [], metadata: nil)[:id]
      # Should not raise
      @db.store_embedding(memory_id, Array.new(384) { rand })

      assert_equal 0, @db.embedding_count
    end

    def test_store_embedding_stores_correctly
      open_vector_database_or_skip
      memory_id = @db.store(content: "test memory", memory_type: "note", tags: [], metadata: nil)[:id]
      embedding = Array.new(384) { rand(-1.0..1.0) }

      @db.store_embedding(memory_id, embedding)

      assert_equal 1, @db.embedding_count
    end

    def test_vector_search_returns_empty_when_disabled
      @db = Database.new(@db_path)
      query = Array.new(384) { rand }

      results = @db.vector_search(query, limit: 10)

      assert_empty results
    end

    def test_vector_search_finds_similar_vectors
      open_vector_database_or_skip

      # Store memories with embeddings
      id1 = @db.store(content: "ruby programming", memory_type: "note", tags: [], metadata: nil)[:id]
      id2 = @db.store(content: "python scripting", memory_type: "note", tags: [], metadata: nil)[:id]
      id3 = @db.store(content: "javascript frontend", memory_type: "note", tags: [], metadata: nil)[:id]
      # Create fake embeddings - all similar to query but with varying degrees
      # Using small noise levels to ensure all results are within the distance threshold
      query_embedding = normalized_vector(384)
      @db.store_embedding(id1, similar_vector(query_embedding, 0.01))  # Very similar
      @db.store_embedding(id2, similar_vector(query_embedding, 0.05))  # Similar
      @db.store_embedding(id3, similar_vector(query_embedding, 0.10))  # Moderately similar

      results = @db.vector_search(query_embedding, limit: 10)

      assert_equal 3, results.length
      # Results should be ordered by distance (closest first)
      assert_operator results[0]["distance"], :<=, results[1]["distance"]
      assert_operator results[1]["distance"], :<=, results[2]["distance"]
    end

    def test_vector_search_includes_memory_fields
      open_vector_database_or_skip
      id = @db.store(content: "test content", memory_type: "decision", tags: ["foo"], metadata: nil)[:id]
      embedding = normalized_vector(384)
      @db.store_embedding(id, embedding)

      results = @db.vector_search(embedding, limit: 1)

      assert_equal 1, results.length
      result = results.first

      assert_equal id, result["id"]
      assert_equal "test content", result["content"]
      assert_equal "decision", result["memory_type"]
      assert_equal ["foo"], result["tags"]
      assert result["distance"]
    end

    def test_embedding_count_returns_zero_when_disabled
      @db = Database.new(@db_path)

      assert_equal 0, @db.embedding_count
    end

    def test_memories_without_embeddings_returns_empty_when_disabled
      @db = Database.new(@db_path)
      @db.store(content: "test", memory_type: "note", tags: [], metadata: nil)

      assert_empty @db.memories_without_embeddings
    end

    def test_memories_without_embeddings_finds_missing
      open_vector_database_or_skip
      id1 = @db.store(content: "has embedding", memory_type: "note", tags: [], metadata: nil)[:id]
      id2 = @db.store(content: "no embedding", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.store_embedding(id1, normalized_vector(384))

      missing = @db.memories_without_embeddings

      assert_equal 1, missing.length
      assert_equal id2, missing.first["id"]
      assert_equal "no embedding", missing.first["content"]
    end

    def test_memories_without_embeddings_excludes_tombstoned
      open_vector_database_or_skip
      alive = @db.store(content: "alive", memory_type: "note", tags: [], metadata: nil)[:id]
      dead = @db.store(content: "dead", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.instance_variable_get(:@db).execute(
        "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
        [Time.now.utc.iso8601, "local", dead]
      )
      ids = @db.memories_without_embeddings(limit: 100).map { |r| r["id"] }

      assert_includes ids, alive
      refute_includes ids, dead
    end

    def test_delete_removes_embedding
      open_vector_database_or_skip
      id = @db.store(content: "will be deleted", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.store_embedding(id, normalized_vector(384))

      assert_equal 1, @db.embedding_count

      @db.delete(id)

      assert_equal 0, @db.embedding_count
    end

    def test_tombstone_removes_row_from_vec_index
      open_vector_database_or_skip
      memory_id = @db.store(content: "tombstone target", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.store_embedding(memory_id, Array.new(384) { rand(-1.0..1.0) })

      assert_equal 1, @db.embedding_count

      raw = @db.instance_variable_get(:@db)
      raw.execute(
        "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
        [Time.now.utc.iso8601, "local", memory_id]
      )

      assert_equal 0, @db.embedding_count
    end

    def test_vector_search_filters_out_low_relevance_results
      open_vector_database_or_skip

      # Store memories with embeddings
      id1 = @db.store(content: "relevant result", memory_type: "note", tags: [], metadata: nil)[:id]
      id2 = @db.store(content: "completely unrelated", memory_type: "note", tags: [], metadata: nil)[:id]
      # Create a query embedding and store embeddings:
      # - id1 gets a similar embedding (low distance, high relevance)
      # - id2 gets an orthogonal/opposite embedding (high distance, low relevance)
      query_embedding = normalized_vector(384)
      @db.store_embedding(id1, similar_vector(query_embedding, 0.1)) # Very similar
      @db.store_embedding(id2, opposite_vector(query_embedding))     # Opposite/unrelated

      results = @db.vector_search(query_embedding, limit: 10)

      # Should only return the relevant result, not the unrelated one
      assert_equal 1, results.length, "Should filter out low-relevance results"
      assert_equal id1, results.first["id"]
    end

    def test_list_includes_has_embedding_when_vectors_enabled
      open_vector_database_or_skip
      id1 = @db.store(content: "has embedding", memory_type: "note", tags: [], metadata: nil)[:id]
      id2 = @db.store(content: "no embedding", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.store_embedding(id1, normalized_vector(384))

      results = @db.list
      mem_with = results.find { |m| m["id"] == id1 }
      mem_without = results.find { |m| m["id"] == id2 }

      assert mem_with["has_embedding"]
      refute mem_without["has_embedding"]
    end

    def test_list_does_not_include_has_embedding_when_vectors_disabled
      @db = Database.new(@db_path)
      @db.store(content: "test", memory_type: "note", tags: [], metadata: nil)

      results = @db.list

      refute results.first.key?("has_embedding")
    end

    def test_get_includes_has_embedding_when_vectors_enabled
      open_vector_database_or_skip
      id = @db.store(content: "test", memory_type: "note", tags: [], metadata: nil)[:id]
      @db.store_embedding(id, normalized_vector(384))

      result = @db.get(id)

      assert result["has_embedding"]
    end

    private

    # Prediction gate: vec_extension_path is only a File.exist? check. Used
    # solely by test_load_vector_extension_succeeds, whose job is to assert
    # that a file on disk actually loads - everything else gates on the
    # database's real post-load state via open_vector_database_or_skip.
    def skip_unless_vec_extension_available
      skip "sqlite-vec not available" unless Recollect.config.vec_extension_path
    end

    # Reality gate: opens the test database with load_vectors: true and skips
    # unless the extension actually loaded. A present-but-unloadable vec0.so
    # skips here; bin/verify-vector-stack is what turns that state into a red
    # nightly before the suite runs.
    def open_vector_database_or_skip
      @db = Database.new(@db_path, load_vectors: true)
      skip "sqlite-vec extension did not load" unless @db.vectors_enabled?

      @db
    end

    def normalized_vector(dimensions)
      vec = Array.new(dimensions) { rand(-1.0..1.0) }
      norm = Math.sqrt(vec.sum { |x| x**2 })
      vec.map { |x| x / norm }
    end

    def similar_vector(base, noise_level)
      vec = base.map { |x| x + rand(-noise_level..noise_level) }
      norm = Math.sqrt(vec.sum { |x| x**2 })
      vec.map { |x| x / norm }
    end

    def opposite_vector(base)
      # Negate the vector to get maximum cosine distance (~2.0)
      base.map { |x| -x }
    end
  end
end
