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

    def test_load_vector_extension_succeeds
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)

      assert_predicate @db, :vectors_enabled?
    end

    def test_store_embedding_noop_when_vectors_disabled
      @db = Database.new(@db_path)
      memory_id = @db.store(content: "test", memory_type: "note", tags: [], metadata: nil)

      # Should not raise
      @db.store_embedding(memory_id, Array.new(384) { rand })

      assert_equal 0, @db.embedding_count
    end

    def test_store_embedding_stores_correctly
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)
      memory_id = @db.store(content: "test memory", memory_type: "note", tags: [], metadata: nil)
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
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)

      # Store memories with embeddings
      id1 = @db.store(content: "ruby programming", memory_type: "note", tags: [], metadata: nil)
      id2 = @db.store(content: "python scripting", memory_type: "note", tags: [], metadata: nil)
      id3 = @db.store(content: "javascript frontend", memory_type: "note", tags: [], metadata: nil)

      # Create fake embeddings - make id1's embedding similar to query
      query_embedding = normalized_vector(384)
      @db.store_embedding(id1, similar_vector(query_embedding, 0.1))  # Very similar
      @db.store_embedding(id2, similar_vector(query_embedding, 0.5))  # Moderately similar
      @db.store_embedding(id3, normalized_vector(384))                # Random, likely dissimilar

      results = @db.vector_search(query_embedding, limit: 10)

      assert_equal 3, results.length
      # Results should be ordered by distance (closest first)
      assert_operator results[0]["distance"], :<=, results[1]["distance"]
      assert_operator results[1]["distance"], :<=, results[2]["distance"]
    end

    def test_vector_search_includes_memory_fields
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)
      id = @db.store(content: "test content", memory_type: "decision", tags: ["foo"], metadata: nil)
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
      skip_unless_vec_extension_available

      @db = Database.new(@db_path, load_vectors: true)
      id1 = @db.store(content: "has embedding", memory_type: "note", tags: [], metadata: nil)
      id2 = @db.store(content: "no embedding", memory_type: "note", tags: [], metadata: nil)

      @db.store_embedding(id1, normalized_vector(384))

      missing = @db.memories_without_embeddings

      assert_equal 1, missing.length
      assert_equal id2, missing.first["id"]
      assert_equal "no embedding", missing.first["content"]
    end

    private

    def skip_unless_vec_extension_available
      skip "sqlite-vec not available" unless Recollect.config.vec_extension_path
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
  end
end
