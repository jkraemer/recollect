# frozen_string_literal: true

module Recollect
  module Sync
    # Background worker that pushes locally-committed records to subscribed peers.
    # MemoriesService#store / #delete enqueue (global_id, db_name); the worker thread
    # looks up the record, finds trusted peers whose peer_db_subscriptions includes
    # the db, and POSTs /sync/push to each.
    #
    # Failures are logged + recorded in known_peers.last_sync_error; the heartbeat
    # (Sync::Engine) catches anything that didn't propagate via push.
    class PushQueue
      def initialize(store:, db_manager:, client_factory:, size: 1000)
        @store = store
        @db_manager = db_manager
        @client_factory = client_factory
        @queue = SizedQueue.new(size)
        @peers = Peers.new(store)
        @drain_mutex = Mutex.new
        @drained = ConditionVariable.new
        @inflight = 0
        @running = false
        @thread = nil
      end

      def start
        return if @running
        @running = true
        @thread = Thread.new { run_loop }
      end

      def stop
        @running = false
        @queue.close
        @thread&.join(5)
        # Any items still sitting in the SizedQueue when close() fired are dropped by
        # pop_one's ClosedQueueError rescue; their @inflight bumps stay forever. Reset
        # the counter so a subsequent start/flush cycle isn't poisoned by phantom jobs.
        @drain_mutex.synchronize do
          @inflight = 0
          @drained.broadcast
        end
      end

      def enqueue(global_id:, db_name:)
        return unless @running
        @drain_mutex.synchronize { @inflight += 1 }
        begin
          @queue << {global_id: global_id, db_name: db_name}
        rescue ClosedQueueError
          # Stop raced us between the @running check and the queue push;
          # undo our @inflight bump so flush() doesn't hang on a phantom job.
          @drain_mutex.synchronize do
            @inflight -= 1
            @drained.broadcast if @inflight.zero?
          end
        end
      end

      def flush(timeout: 5)
        deadline = Time.now + timeout
        @drain_mutex.synchronize do
          while @inflight.positive? && Time.now < deadline
            @drained.wait(@drain_mutex, deadline - Time.now)
          end
        end
      end

      private

      def run_loop
        while @running
          job = pop_one
          break if job.nil?
          process(job)
          @drain_mutex.synchronize do
            @inflight -= 1
            @drained.broadcast if @inflight.zero?
          end
        end
      end

      def pop_one
        @queue.pop
      rescue ClosedQueueError
        nil
      end

      def process(job)
        project = @db_manager.project_for_db_name(job[:db_name])
        database = @db_manager.get_database(project)
        row = database.instance_variable_get(:@db).get_first_row(
          "SELECT #{Database::SYNC_COLUMNS} FROM memories WHERE global_id = ?", job[:global_id]
        )
        return unless row
        record = row.except("id")

        @peers.list.each do |peer|
          next unless peer[:status] == "trusted"
          next unless @peers.subscriptions(peer[:peer_id]).include?(job[:db_name])
          client = @client_factory.call(peer)
          begin
            response = client.post_json("/sync/push?db=#{job[:db_name]}", {records: [record]})
            ok = response.respond_to?(:success?) ? response.success? : (200..299).cover?(response.status)
            update_peer_status(peer[:peer_id], success: ok, error: ok ? nil : "HTTP #{response.status}")
          rescue => e
            update_peer_status(peer[:peer_id], success: false, error: e.message)
          end
        end
      end

      def update_peer_status(peer_id, success:, error: nil)
        if success
          @store.instance_variable_get(:@db).execute(
            "UPDATE known_peers SET last_sync_at = ?, last_sync_error = NULL, last_seen_at = ? WHERE peer_id = ?",
            [Time.now.utc.iso8601, Time.now.utc.iso8601, peer_id]
          )
        else
          @store.instance_variable_get(:@db).execute(
            "UPDATE known_peers SET last_sync_error = ? WHERE peer_id = ?", [error, peer_id]
          )
        end
      end
    end
  end
end
