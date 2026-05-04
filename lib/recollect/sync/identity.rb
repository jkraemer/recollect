# frozen_string_literal: true

require "ed25519"
require "base58"
require "digest"
require "socket"

module Recollect
  module Sync
    class Identity
      attr_reader :peer_id, :display_name, :public_key, :private_key, :created_at

      def initialize(peer_id:, display_name:, public_key:, private_key:, created_at:)
        @peer_id = peer_id
        @display_name = display_name
        @public_key = public_key
        @private_key = private_key
        @created_at = created_at
      end

      def self.ensure!(store, display_name: nil)
        existing = load(store)
        return existing if existing

        signing_key = Ed25519::SigningKey.generate
        public_key = signing_key.verify_key.to_bytes
        private_key = signing_key.to_bytes
        peer_id = derive_peer_id(public_key)
        name = display_name || Socket.gethostname

        store.instance_variable_get(:@db).execute(
          "INSERT INTO local_identity (peer_id, display_name, public_key, private_key) VALUES (?, ?, ?, ?)",
          [peer_id, name, SQLite3::Blob.new(public_key), SQLite3::Blob.new(private_key)]
        )
        load(store)
      end

      def self.load(store)
        row = store.instance_variable_get(:@db).get_first_row("SELECT * FROM local_identity LIMIT 1")
        return nil unless row

        new(
          peer_id: row["peer_id"],
          display_name: row["display_name"],
          public_key: row["public_key"],
          private_key: row["private_key"],
          created_at: row["created_at"]
        )
      end

      def self.derive_peer_id(public_key)
        Base58.binary_to_base58(Digest::SHA256.digest(public_key)[0, 16].b)
      end
    end
  end
end
