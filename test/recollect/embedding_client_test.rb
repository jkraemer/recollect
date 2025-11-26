# frozen_string_literal: true

require "test_helper"

module Recollect
  class EmbeddingClientTest < TestCase
    def setup
      super
      @client = EmbeddingClient.new
    end

    def teardown
      @client&.shutdown
      super
    end

    def test_embed_returns_array_of_floats
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
      config = Recollect.config.dup
      config.embed_server_script_path = Pathname.new("/nonexistent/script")

      def config.python_path
        "python3"
      end

      client = EmbeddingClient.new(config: config)

      assert_raises(EmbeddingClient::EmbeddingError) do
        client.embed("test")
      end
    ensure
      client&.shutdown
    end

    def test_healthy_returns_true_when_process_running
      skip_unless_vectors_available

      # Start the process first
      @client.embed("warmup")

      assert_predicate @client, :healthy?
    end

    def test_healthy_returns_false_when_process_not_started
      # Client was just created, process not started yet
      refute_predicate @client, :healthy?
    end

    def test_embeddings_are_normalized
      skip_unless_vectors_available

      embedding = @client.embed("test normalization")

      # L2 norm should be approximately 1.0 for normalized vectors
      l2_norm = Math.sqrt(embedding.sum { |x| x**2 })

      assert_in_delta 1.0, l2_norm, 0.01
    end

    def test_subsequent_requests_are_fast
      skip_unless_vectors_available

      # First request starts the process (slow due to model loading)
      @client.embed("warmup")

      # Subsequent requests should be fast
      start_time = Time.now
      @client.embed("test 1")
      @client.embed("test 2")
      @client.embed("test 3")
      elapsed = Time.now - start_time

      # 3 requests should complete in under 2 seconds (they're ~100ms each)
      assert_operator elapsed, :<, 2.0, "Subsequent requests should be fast (got #{elapsed}s)"
    end

    def test_shutdown_stops_process
      skip_unless_vectors_available

      @client.embed("warmup")

      assert_predicate @client, :healthy?

      @client.shutdown

      refute_predicate @client, :healthy?
    end

    def test_process_respawns_after_crash
      skip_unless_vectors_available

      # Start the process
      embedding1 = @client.embed("before crash")

      assert_equal 384, embedding1.length

      # Simulate crash by killing the process
      @client.shutdown

      # Next request should respawn the process
      embedding2 = @client.embed("after crash")

      assert_equal 384, embedding2.length
    end

    private

    def skip_unless_vectors_available
      venv_python = Recollect.root.join(".venv", "bin", "python3")
      skip "Python venv not available" unless venv_python.executable?

      embed_server = Recollect.root.join("bin", "embed-server")
      skip "embed-server not available" unless embed_server.executable?
    end
  end
end
