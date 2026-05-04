# frozen_string_literal: true

require "ed25519"
require "base64"
require "digest"
require "time"

module Recollect
  module Sync
    module Crypto
      DEFAULT_SKEW_SECONDS = 300

      module_function

      def sign(private_key:, peer_id:, timestamp:, body:)
        sk = Ed25519::SigningKey.new(private_key)
        Base64.strict_encode64(sk.sign(payload(peer_id, timestamp, body)))
      end

      def verify(public_key:, peer_id:, timestamp:, body:, signature:, skew_seconds: DEFAULT_SKEW_SECONDS)
        return false unless within_skew?(timestamp, skew_seconds)

        vk = Ed25519::VerifyKey.new(public_key)
        vk.verify(Base64.strict_decode64(signature), payload(peer_id, timestamp, body))
      rescue Ed25519::VerifyError, ArgumentError
        false
      end

      def payload(peer_id, timestamp, body)
        "#{peer_id}\n#{timestamp}\n#{Digest::SHA256.hexdigest(body || "")}"
      end

      def within_skew?(timestamp, skew_seconds)
        ts = Time.iso8601(timestamp.to_s)
        (Time.now.utc - ts).abs <= skew_seconds
      rescue ArgumentError
        false
      end
    end
  end
end
