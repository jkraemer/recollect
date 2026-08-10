# frozen_string_literal: true

module Recollect
  class EmbeddingWorker
    BATCH_SIZE = 10
    BATCH_WAIT = 2 # seconds to wait for batch to fill
    # Bounds memory if the embedder hangs or falls behind; dropped jobs are
    # recoverable via vector-backfill.
    MAX_QUEUE_SIZE = 10_000

    def initialize(db_manager, queue_size: MAX_QUEUE_SIZE)
      @db_manager = db_manager
      @queue = SizedQueue.new(queue_size)
      @running = false
      @thread = nil
      @client = EmbeddingClient.new
    end

    def start
      return if @running

      @running = true
      @thread = Thread.new { run_loop }
      recover_missing_embeddings
    end

    def stop
      @running = false
      @queue.close
      @thread&.join(5)
      @client.shutdown
    end

    def enqueue(memory_id:, content:, project:)
      return unless @running

      @queue.push({memory_id: memory_id, content: content, project: project}, true)
    rescue ThreadError
      # Queue full. Dropping beats blocking the caller's request thread or
      # growing without bound; vector-backfill re-queues anything missed.
      warn "[EmbeddingWorker] Queue full, dropping embedding job for ##{memory_id} (recover with vector-backfill)"
    end

    def queue_size
      @queue.size
    end

    def running?
      @running && @thread&.alive?
    end

    private

    def run_loop
      while @running
        batch = collect_batch
        process_batch(batch) unless batch.empty?
      end
    rescue => e
      warn "[EmbeddingWorker] Worker loop crashed: #{e.message}"
    end

    def collect_batch
      batch = []
      deadline = Time.now + BATCH_WAIT

      while batch.size < BATCH_SIZE && Time.now < deadline
        remaining = deadline - Time.now
        break if remaining <= 0

        begin
          # Use pop with timeout
          item = @queue.pop(timeout: [remaining, 0.1].min)
          # pop(timeout:) returns nil both on a plain timeout (queue still
          # open, keep waiting for the batch to fill) and once a closed
          # queue is drained (it never raises ThreadError for the timed
          # form) -- @queue.closed? is the only way to tell those apart.
          break if item.nil? && @queue.closed?

          batch << item if item
        rescue ThreadError
          # Queue closed
          break
        end
      end

      batch
    end

    def process_batch(batch)
      texts = batch.map { |item| item[:content] }

      embeddings = @client.embed_batch(texts)

      if embeddings.length != batch.length
        warn "[EmbeddingWorker] Embedding count mismatch: expected #{batch.length}, got #{embeddings.length}. Skipping batch."
        return
      end

      batch.zip(embeddings).each do |item, embedding|
        store_embedding(item, embedding)
      end
    rescue EmbeddingClient::EmbeddingError => e
      warn "[EmbeddingWorker] Batch failed: #{e.message}"
    end

    def store_embedding(item, embedding)
      db = @db_manager.get_database(item[:project])
      db.store_embedding(item[:memory_id], embedding)
    rescue => e
      warn "[EmbeddingWorker] Failed to store embedding for ##{item[:memory_id]}: #{e.message}"
    end

    def recover_missing_embeddings
      total = 0
      # Include global database (nil) plus all project databases
      projects = [nil] + @db_manager.list_projects
      projects.each do |project|
        db = @db_manager.get_database(project)
        db.memories_without_embeddings.each do |row|
          enqueue(memory_id: row["id"], content: row["content"], project: project)
          total += 1
        end
      end
      warn "[EmbeddingWorker] Recovering #{total} missing embeddings" if total.positive?
    rescue => e
      warn "[EmbeddingWorker] Recovery failed: #{e.message}"
    end
  end
end
