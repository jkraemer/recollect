# frozen_string_literal: true

require "faraday"
require "json"
require "time"

module Recollect
  module Sync
    # Outbound HTTP client for sync requests. Signs every request with the local Ed25519
    # private key so the receiving peer's signature middleware can verify authenticity.
    #
    # Response contract: returns a Faraday::Response. Does NOT raise on non-2xx — callers
    # must check response.status. Network failures raise Faraday::Error subclasses.
    #
    # The adapter is pluggable so integration tests can use [:rack, app] to wire two
    # HTTPServer instances together in-process. Production uses [:net_http].
    class Client
      def initialize(peer_id:, private_key:, endpoint:, adapter: nil)
        @peer_id = peer_id
        @private_key = private_key
        @endpoint = endpoint
        @adapter = adapter || [:net_http]
        @conn = Faraday.new(url: endpoint) { |f| f.adapter(*@adapter) }
      end

      def get(path)
        send_signed(:get, path, "")
      end

      def post_json(path, payload)
        body = JSON.generate(payload)
        send_signed(:post, path, body, content_type: "application/json")
      end

      private

      def send_signed(method, path, body, content_type: nil)
        ts = Time.now.utc.iso8601
        sig = Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: ts, body: body)
        @conn.run_request(method, path, body, nil) do |req|
          req.headers["Content-Type"] = content_type if content_type
          req.headers["X-Peer-ID"] = @peer_id
          req.headers["X-Timestamp"] = ts
          req.headers["X-Signature"] = sig
        end
      end
    end
  end
end
