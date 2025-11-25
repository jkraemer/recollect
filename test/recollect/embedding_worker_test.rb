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
      @worker.start

      @worker.enqueue(memory_id: 1, content: "test", project: nil)

      assert_equal 1, @worker.queue_size
    end

    def test_enqueue_ignored_when_not_running
      # Worker not started
      @worker.enqueue(memory_id: 1, content: "test", project: nil)

      assert_equal 0, @worker.queue_size
    end

    def test_worker_processes_queue_items
      skip_unless_vectors_available

      # Store a memory first so we have something to attach an embedding to
      db = @db_manager.get_database(nil)
      memory_id = db.store(content: "test memory", memory_type: "note", tags: [], metadata: nil, source: "test")

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
        db.store(content: "memory #{i}", memory_type: "note", tags: [], metadata: nil, source: "test")
      end

      @worker.start
      ids.each_with_index do |id, i|
        @worker.enqueue(memory_id: id, content: "memory #{i}", project: nil)
      end

      # Wait for batch processing
      sleep 4

      assert_equal 0, @worker.queue_size
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
