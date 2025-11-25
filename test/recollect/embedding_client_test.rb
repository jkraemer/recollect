# frozen_string_literal: true

require "test_helper"

module Recollect
  class EmbeddingClientTest < TestCase
    def setup
      super
      @client = EmbeddingClient.new
    end

    def test_embed_returns_array_of_floats
      # This test uses the real Python script - skip if not available
      skip_unless_vectors_available

      embedding = @client.embed("hello world")

      assert_kind_of Array, embedding
      assert_equal 384, embedding.length
      assert_kind_of Float, embedding.first
    end

    def test_embed_batch_processes_multiple_texts
      skip_unless_vectors_available

      embeddings = @client.embed_batch(%w[hello world])

      assert_equal 2, embeddings.length
      assert_equal 384, embeddings[0].length
      assert_equal 384, embeddings[1].length
    end

    def test_embed_batch_returns_empty_array_for_empty_input
      embeddings = @client.embed_batch([])

      assert_empty embeddings
    end

    def test_embedding_error_raised_on_script_failure
      # Create a client with a bad script path
      config = Recollect.config.dup
      config.embed_script_path = Pathname.new("/nonexistent/script")

      # Also need to mock python_path to something that exists
      def config.python_path
        "python3"
      end

      client = EmbeddingClient.new(config: config)

      assert_raises(EmbeddingClient::EmbeddingError) do
        client.embed("test")
      end
    end

    def test_healthy_returns_true_when_script_works
      skip_unless_vectors_available

      assert_predicate @client, :healthy?
    end

    def test_healthy_returns_false_when_script_fails
      config = Recollect.config.dup
      config.embed_script_path = Pathname.new("/nonexistent/script")

      def config.python_path
        "python3"
      end

      client = EmbeddingClient.new(config: config)

      refute_predicate client, :healthy?
    end

    def test_embeddings_are_normalized
      skip_unless_vectors_available

      embedding = @client.embed("test normalization")

      # L2 norm should be approximately 1.0 for normalized vectors
      l2_norm = Math.sqrt(embedding.sum { |x| x**2 })

      assert_in_delta 1.0, l2_norm, 0.01
    end

    private

    def skip_unless_vectors_available
      venv_python = Recollect.root.join(".venv", "bin", "python3")
      skip "Python venv not available" unless venv_python.executable?
    end
  end
end
