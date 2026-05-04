# frozen_string_literal: true

require "sqlite3"
require "fileutils"

module Recollect
  module Sync
    class Store
      SCHEMA = <<~SQL
        CREATE TABLE IF NOT EXISTS local_identity (
          peer_id      TEXT PRIMARY KEY,
          display_name TEXT,
          public_key   BLOB NOT NULL,
          private_key  BLOB NOT NULL,
          created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        );

        CREATE TABLE IF NOT EXISTS known_peers (
          peer_id         TEXT PRIMARY KEY,
          display_name    TEXT,
          public_key      BLOB NOT NULL,
          endpoint        TEXT NOT NULL,
          status          TEXT NOT NULL DEFAULT 'trusted',
          trusted_at      TEXT NOT NULL,
          last_seen_at    TEXT,
          last_sync_at    TEXT,
          last_sync_error TEXT
        );

        CREATE TABLE IF NOT EXISTS peer_watermarks (
          peer_id           TEXT NOT NULL,
          db_name           TEXT NOT NULL,
          latest_created_at TEXT NOT NULL,
          PRIMARY KEY (peer_id, db_name)
        );

        CREATE TABLE IF NOT EXISTS pairing_codes (
          code         TEXT PRIMARY KEY,
          created_at   TEXT NOT NULL,
          expires_at   TEXT NOT NULL,
          used_at      TEXT,
          used_by_peer TEXT
        );

        CREATE TABLE IF NOT EXISTS peer_db_subscriptions (
          peer_id TEXT NOT NULL,
          db_name TEXT NOT NULL,
          PRIMARY KEY (peer_id, db_name),
          FOREIGN KEY (peer_id) REFERENCES known_peers(peer_id) ON DELETE CASCADE
        );
      SQL

      def initialize(path)
        @path = path.to_s
        FileUtils.mkdir_p(File.dirname(@path))
        @db = SQLite3::Database.new(@path)
        @db.results_as_hash = true
        @db.execute("PRAGMA journal_mode = WAL")
        @db.execute("PRAGMA foreign_keys = ON")
        @db.execute_batch(SCHEMA)
        File.chmod(0o600, @path)
      end

      def close
        @db&.close
        @db = nil
      end
    end
  end
end
