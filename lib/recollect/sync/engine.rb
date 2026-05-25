# frozen_string_literal: true

module Recollect
  module Sync
    # Background sync orchestrator. #reconcile(peer:, db_name:) does the
    # manifest -> pull (paginated) -> push exchange for one (peer, db).
    #
    # All cursors are composite {created_at, global_id} per Errata.
    class Engine
      PULL_LIMIT = 500

      def initialize(store:, db_manager:, client_factory:, heartbeat_seconds: 300)
        @store = store
        @db_manager = db_manager
        @client_factory = client_factory
        @heartbeat_seconds = heartbeat_seconds
        @watermarks = Watermarks.new(store)
        @peers = Peers.new(store)
        @running = false
        @thread = nil
      end

      def reconcile(peer:, db_name:)
        client = @client_factory.call(peer)

        # Step 1: peer's manifest (what they have).
        manifest_response = client.get("/sync/manifest?db=#{db_name}")
        peer_watermarks = JSON.parse(manifest_response.body)["watermarks"]

        # Step 2: pull records we're missing.
        pull_loop(client: client, db_name: db_name)

        # Step 3: push records peer is missing (from any origin we know about).
        push_missing(client: client, db_name: db_name, peer_watermarks: peer_watermarks)

        update_peer_success(peer[:peer_id])
      rescue => e
        update_peer_error(peer[:peer_id], e.message)
      end

      private

      def pull_loop(client:, db_name:)
        project = @db_manager.project_for_db_name(db_name)
        database = @db_manager.get_database(project)

        loop do
          since = @watermarks.get(db_name: db_name)
          response = client.post_json("/sync/pull?db=#{db_name}", {since: since, limit: PULL_LIMIT})
          page = JSON.parse(response.body)["records"]
          break if page.empty?

          observed = Hash.new { |h, k| h[k] = {"created_at" => "", "global_id" => ""} }
          page.each do |rec|
            status = database.upsert_synced(rec)
            update_observed(observed, rec)
            if status == :inserted && rec["memory_type"] != "_chunk" && rec["deleted_at"].nil?
              row = database.instance_variable_get(:@db).get_first_row(
                "SELECT id FROM memories WHERE global_id = ?", rec["global_id"]
              )
              @db_manager.enqueue_embedding(memory_id: row["id"], content: rec["content"], project: project) if row
            end
          end

          observed.each do |origin, cursor|
            next if cursor["created_at"].empty?
            @watermarks.advance(peer_id: origin, db_name: db_name,
              created_at: cursor["created_at"], global_id: cursor["global_id"])
          end

          break if page.size < PULL_LIMIT
        end
      end

      def push_missing(client:, db_name:, peer_watermarks:)
        project = @db_manager.project_for_db_name(db_name)
        database = @db_manager.get_database(project)
        rows = database.fetch_for_sync(since: peer_watermarks, limit: PULL_LIMIT)
        return if rows.empty?

        records = rows.map { |r| r.except("id") }
        client.post_json("/sync/push?db=#{db_name}", {records: records})
      end

      def update_observed(observed, rec)
        origin = rec["origin_peer"]
        current = [observed[origin]["created_at"], observed[origin]["global_id"]]
        incoming = [rec["created_at"], rec["global_id"]]
        if (incoming <=> current) == 1
          observed[origin] = {"created_at" => rec["created_at"], "global_id" => rec["global_id"]}
        end
      end

      def update_peer_success(peer_id)
        @store.instance_variable_get(:@db).execute(
          "UPDATE known_peers SET last_sync_at = ?, last_sync_error = NULL, last_seen_at = ? WHERE peer_id = ?",
          [Time.now.utc.iso8601, Time.now.utc.iso8601, peer_id]
        )
      end

      def update_peer_error(peer_id, message)
        @store.instance_variable_get(:@db).execute(
          "UPDATE known_peers SET last_sync_error = ? WHERE peer_id = ?", [message, peer_id]
        )
      end
    end
  end
end
