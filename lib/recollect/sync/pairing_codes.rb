# frozen_string_literal: true

require "securerandom"
require "time"

module Recollect
  module Sync
    class PairingCodes
      ALPHABET = (("A".."Z").to_a + ("0".."9").to_a).freeze

      def initialize(store)
        @store = store
      end

      def generate(ttl_seconds: 300)
        code = "#{rand_block}-#{rand_block}"
        now = Time.now.utc
        exp = now + ttl_seconds
        db.execute(
          "INSERT INTO pairing_codes (code, created_at, expires_at) VALUES (?, ?, ?)",
          [code, now.iso8601, exp.iso8601]
        )
        {code: code, expires_at: exp}
      end

      def consume(code, used_by_peer:)
        now = Time.now.utc.iso8601
        db.execute(
          "UPDATE pairing_codes SET used_at = ?, used_by_peer = ? " \
          "WHERE code = ? AND used_at IS NULL AND expires_at > ?",
          [now, used_by_peer, code, now]
        )
        db.changes.positive?
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end

      def rand_block
        Array.new(4) { ALPHABET.sample(random: SecureRandom) }.join
      end
    end
  end
end
