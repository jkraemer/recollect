# frozen_string_literal: true

module Recollect
  class EmbeddingWorker
    BATCH_SIZE = 10
    BATCH_WAIT = 2 # seconds to wait for batch to fill

    def initialize(db_manager)
      @db_manager = db_manager
      @queue = Queue.new
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

      @queue << {memory_id: memory_id, content: content, project: project}
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
