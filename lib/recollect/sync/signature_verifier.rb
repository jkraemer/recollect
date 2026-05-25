# frozen_string_literal: true

module Recollect
  module Sync
    class SignatureVerifier
      def initialize(store)
        @peers = Peers.new(store)
      end

      # headers may be a Rack env (with HTTP_X_*) or a hash with simple keys.
      def verify(headers:, body:)
        peer_id = headers["HTTP_X_PEER_ID"] || headers["X-Peer-ID"]
        timestamp = headers["HTTP_X_TIMESTAMP"] || headers["X-Timestamp"]
        signature = headers["HTTP_X_SIGNATURE"] || headers["X-Signature"]
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
    end
  end
end
