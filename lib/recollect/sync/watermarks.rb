# frozen_string_literal: true

module Recollect
  module Sync
    # CRUD over the peer_watermarks table.
    #
    # Watermarks are composite (created_at, global_id) cursors, one per (peer_id, db_name).
    # Read/write shape matches the wire format used by /sync/manifest and /sync/pull:
    #   { peer_id => { "created_at" => iso8601, "global_id" => uuid } }
    #
    # Advance takes MAX semantics on the composite tuple: a new cursor replaces the stored
    # one only if (new_ts, new_gid) is strictly greater lex than (old_ts, old_gid).
    class Watermarks
      def initialize(store)
        @store = store
      end

      # Returns { peer_id => { "created_at" => ..., "global_id" => ... } } for the given db.
      # Empty hash when no rows exist.
      def get(db_name:)
        rows = db.execute(
          "SELECT peer_id, latest_created_at, latest_global_id FROM peer_watermarks WHERE db_name = ?",
          db_name
        )
        rows.each_with_object({}) do |row, h|
          h[row["peer_id"]] = {"created_at" => row["latest_created_at"], "global_id" => row["latest_global_id"]}
        end
      end

      # Inserts or updates the cursor for (peer_id, db_name). The composite (created_at, global_id)
      # only moves forward; supplying an older or equal cursor is a no-op.
      #
      # global_id must be non-nil. SQLite three-valued logic would make (ts, NULL) > (ts, 'g-x')
      # evaluate to NULL, silently falling through to "keep old" and masking the bug. The schema
      # column is nullable only to permit in-place ALTER on existing peer_watermarks rows.
      def advance(peer_id:, db_name:, created_at:, global_id:)
        raise ArgumentError, "created_at must not be nil" if created_at.nil?
        raise ArgumentError, "global_id must not be nil" if global_id.nil?

        db.execute(
          "INSERT INTO peer_watermarks (peer_id, db_name, latest_created_at, latest_global_id) " \
          "VALUES (?, ?, ?, ?) " \
          "ON CONFLICT(peer_id, db_name) DO UPDATE SET " \
          "latest_created_at = CASE " \
          "  WHEN (excluded.latest_created_at, excluded.latest_global_id) > " \
          "       (peer_watermarks.latest_created_at, peer_watermarks.latest_global_id) " \
          "  THEN excluded.latest_created_at " \
          "  ELSE peer_watermarks.latest_created_at END, " \
          "latest_global_id = CASE " \
          "  WHEN (excluded.latest_created_at, excluded.latest_global_id) > " \
          "       (peer_watermarks.latest_created_at, peer_watermarks.latest_global_id) " \
          "  THEN excluded.latest_global_id " \
          "  ELSE peer_watermarks.latest_global_id END",
          [peer_id, db_name, created_at, global_id]
        )
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end
    end
  end
end
