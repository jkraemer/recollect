# frozen_string_literal: true

require "test_helper"

module Recollect
  class EmbeddingWorkerTest < TestCase
    def setup
      super
      @db_manager = DatabaseManager.new
      @worker = EmbeddingWorker.new(@db_manager)
    end

    def teardown
      @worker&.stop
      @db_manager&.close_all
      super
    end

    def test_start_and_stop
      refute_predicate @worker, :running?

      @worker.start

      assert_predicate @worker, :running?

      @worker.stop

      refute_predicate @worker, :running?
    end

    def test_enqueue_adds_to_queue
      # Mark running without starting the consumer thread: this tests queue
      # admission only. A live consumer races the size assertion and, on
      # machines without the vector stack, fails the batch asynchronously -
      # stderr noise no capture_io around this body can reliably catch.
      @worker.instance_variable_set(:@running, true)

      @worker.enqueue(memory_id: 1, content: "test", project: nil)

      assert_equal 1, @worker.queue_size
    end

    def test_enqueue_drops_new_jobs_when_queue_full
      worker = EmbeddingWorker.new(@db_manager, queue_size: 2)
      # Mark running without starting the consumer thread, so the queue
      # genuinely fills up (mirrors a hung Python embedder).
      worker.instance_variable_set(:@running, true)

      _, err = capture_io do
        3.times { |i| worker.enqueue(memory_id: i, content: "x", project: nil) }
      end

      assert_equal 2, worker.queue_size
      assert_match(/dropping/i, err)
    ensure
      worker&.stop
    end

    def test_enqueue_ignored_when_not_running
      # Worker not started
      @worker.enqueue(memory_id: 1, content: "test", project: nil)

      assert_equal 0, @worker.queue_size
    end

    def test_stop_returns_promptly_when_queue_closed_and_empty
      # SizedQueue#pop(timeout:) returns nil on a closed queue -- it never
      # raises ThreadError, so collect_batch's rescue is dead code. Without a
      # closed-queue check, stop busy-spins in the background thread until
      # collect_batch's BATCH_WAIT deadline, burning a full CPU core.
      @worker.start
      sleep 0.05 # let the background thread enter collect_batch's wait window

      start_time = Time.now
      @worker.stop
      elapsed = Time.now - start_time

      assert_operator elapsed, :<, EmbeddingWorker::BATCH_WAIT / 2.0,
        "stop should return promptly once the queue is closed and drained, not busy-spin " \
        "for the remaining BATCH_WAIT window (took #{elapsed.round(3)}s)"
    end

    def test_worker_processes_queue_items
      skip_unless_vectors_available

      # Store a memory first so we have something to attach an embedding to
      db = @db_manager.get_database(nil)
      memory_id = db.store(content: "test memory", memory_type: "note", tags: [], metadata: nil)[:id]
      @worker.start
      @worker.enqueue(memory_id: memory_id, content: "test memory", project: nil)

      # Wait for processing
      sleep 3

      # The embedding should have been stored (we'll verify this in Phase 5 tests)
      # For now just verify the queue was processed
      assert_equal 0, @worker.queue_size
    end

    def test_worker_batches_multiple_items
      skip_unless_vectors_available

      db = @db_manager.get_database(nil)
      ids = 3.times.map do |i|
        db.store(content: "memory #{i}", memory_type: "note", tags: [], metadata: nil)
      end

      @worker.start
      ids.each_with_index do |id, i|
        @worker.enqueue(memory_id: id, content: "memory #{i}", project: nil)
      end

      # Wait for batch processing
      sleep 4

      assert_equal 0, @worker.queue_size
    end

    def test_start_recovers_missing_embeddings
      skip_unless_vectors_available

      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        # Create fresh db_manager with vectors enabled
        config = Config.new
        @db_manager&.close_all
        @worker&.stop
        @db_manager = DatabaseManager.new(config)
        @worker = EmbeddingWorker.new(@db_manager)

        # Store a memory directly in the database (bypassing the worker)
        db = @db_manager.get_database(nil)
        db.store(content: "orphaned memory", memory_type: "note", tags: [], metadata: nil)

        # Verify it has no embedding
        assert_equal 1, db.memories_without_embeddings.size

        # Start worker - should detect and recover missing embedding
        @worker.start

        # Wait for recovery and processing (model loading can take 20+ seconds on cold start)
        30.times do
          break if db.memories_without_embeddings.empty?

          sleep 1
        end

        # Embedding should now exist
        assert_equal 0, db.memories_without_embeddings.size
      end
    end

    def test_process_batch_validates_embedding_count
      # This tests that mismatched embedding counts are detected and logged
      # rather than silently misaligning embeddings with memories

      db = @db_manager.get_database(nil)
      id1 = db.store(content: "memory one", memory_type: "note", tags: [], metadata: nil)[:id]
      id2 = db.store(content: "memory two", memory_type: "note", tags: [], metadata: nil)[:id]
      batch = [
        {memory_id: id1, content: "memory one", project: nil},
        {memory_id: id2, content: "memory two", project: nil}
      ]

      # Mock client that returns wrong number of embeddings (1 instead of 2)
      mock_client = Minitest::Mock.new
      mock_client.expect(:embed_batch, [[0.1] * 384], [%w[memory\ one memory\ two]])

      original_client = @worker.instance_variable_get(:@client)
      warnings = []
      begin
        @worker.stub(:warn, ->(msg) { warnings << msg }) do
          @worker.instance_variable_set(:@client, mock_client)
          @worker.send(:process_batch, batch)
        end
      ensure
        @worker.instance_variable_set(:@client, original_client)
      end

      mock_client.verify

      # Should have logged a warning about the mismatch
      assert warnings.any? { |w| w.include?("mismatch") },
        "Expected warning about count mismatch, got: #{warnings.inspect}"
    end

    private

    def skip_unless_vectors_available
      return if Recollect.config.vectors_available?

      skip "Vector stack not configured (#{Recollect.config.vector_status_message})"
    end
  end
end
