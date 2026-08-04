# frozen_string_literal: true

module Recollect
  module Sync
    class SignatureVerifier
      def initialize(store)
        @peers = Peers.new(store)
      end

      # headers may be a Rack env (with HTTP_X_*) or a hash with simple keys.
      def verify(headers:, body:)
        peer_id = header_value(headers, "HTTP_X_PEER_ID", "X-Peer-ID")
        timestamp = header_value(headers, "HTTP_X_TIMESTAMP", "X-Timestamp")
        signature = header_value(headers, "HTTP_X_SIGNATURE", "X-Signature")
        return {status: :unauthorized, reason: "missing-headers"} if [peer_id, timestamp, signature].any?(&:nil?)

        peer = @peers.find(peer_id)
        return {status: :forbidden, reason: "unknown-peer"} unless peer
        return {status: :forbidden, reason: "blocked-peer"} if peer[:status] == "blocked"

        valid = Crypto.verify(
          public_key: peer[:public_key], peer_id: peer_id, timestamp: timestamp, body: body, signature: signature
        )
        return {status: :unauthorized, reason: "bad-signature"} unless valid

        {status: :ok, peer: peer}
      end

      private

      # Rack delivers header values as ASCII-8BIT strings, which SQLite binds
      # as BLOBs that never match TEXT columns; normalize to UTF-8.
      def header_value(headers, *keys)
        value = keys.filter_map { |k| headers[k] }.first
        value&.dup&.force_encoding(Encoding::UTF_8)
      end
    end
  end
end
