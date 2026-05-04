# frozen_string_literal: true

require "time"

module Recollect
  module Sync
    class Peers
      def initialize(store)
        @store = store
      end

      def add(peer_id:, display_name:, public_key:, endpoint:, default_subscription: nil)
        db.execute(
          "INSERT INTO known_peers (peer_id, display_name, public_key, endpoint, status, trusted_at) " \
          "VALUES (?, ?, ?, ?, 'trusted', ?) " \
          "ON CONFLICT(peer_id) DO UPDATE SET " \
          "display_name = excluded.display_name, " \
          "public_key = excluded.public_key, " \
          "endpoint = excluded.endpoint, " \
          "status = 'trusted'",
          [peer_id, display_name, SQLite3::Blob.new(public_key), endpoint, Time.now.utc.iso8601]
        )
        subscribe(peer_id, default_subscription) if default_subscription
      end

      def find(peer_id)
        row = db.get_first_row("SELECT * FROM known_peers WHERE peer_id = ?", peer_id)
        row && symbolize(row)
      end

      def list
        db.execute("SELECT * FROM known_peers ORDER BY peer_id").map { |r| symbolize(r) }
      end

      def block(peer_id)
        db.execute("UPDATE known_peers SET status = 'blocked' WHERE peer_id = ?", peer_id)
      end

      def subscribe(peer_id, db_name)
        db.execute("INSERT OR IGNORE INTO peer_db_subscriptions (peer_id, db_name) VALUES (?, ?)", [peer_id, db_name])
      end

      def unsubscribe(peer_id, db_name)
        db.execute("DELETE FROM peer_db_subscriptions WHERE peer_id = ? AND db_name = ?", [peer_id, db_name])
      end

      def subscriptions(peer_id)
        db.execute("SELECT db_name FROM peer_db_subscriptions WHERE peer_id = ? ORDER BY db_name", peer_id).map { |r| r["db_name"] }
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end

      def symbolize(row)
        {peer_id: row["peer_id"], display_name: row["display_name"], public_key: row["public_key"],
         endpoint: row["endpoint"], status: row["status"], trusted_at: row["trusted_at"],
         last_seen_at: row["last_seen_at"], last_sync_at: row["last_sync_at"], last_sync_error: row["last_sync_error"]}
      end
    end
  end
end
