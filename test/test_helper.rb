# frozen_string_literal: true

# SimpleCov must be started before any app code is loaded
if ENV["COVERAGE"]
  require "simplecov"
  require "simplecov-console"
  SimpleCov.start do
    add_filter "/test/"
    add_filter "/vendor/"
    enable_coverage :branch
    minimum_coverage line: 80, branch: 70
    formatter SimpleCov::Formatter::Console
  end
end

ENV["RACK_ENV"] = "test"

# Store test data directory in constant before any tests modify ENV
TEST_DATA_DIR = File.join(__dir__, "tmp", "test_data")
ENV["RECOLLECT_DATA_DIR"] = TEST_DATA_DIR

require "bundler/setup"
require "mcp"
require "recollect"
require "minitest/autorun"
require "rack/test"
require "fileutils"
require "json"
require "ed25519"

# Ensure test data directory exists
FileUtils.mkdir_p(TEST_DATA_DIR)

module Recollect
  class TestCase < Minitest::Test
    include Rack::Test::Methods

    def setup
      # Reset HTTP server singletons to ensure fresh state
      HTTPServer.reset_db_manager!

      # Clean databases between tests
      Dir.glob(File.join(TEST_DATA_DIR, "**/*.db*")).each do |f|
        FileUtils.rm_f(f)
      end
    end

    def teardown
      # Subclasses can override
    end

    def setup_trusted_peer(peer_id)
      signing_key = Ed25519::SigningKey.generate
      Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).add(
        peer_id: peer_id, display_name: peer_id,
        public_key: signing_key.verify_key.to_bytes, endpoint: "http://#{peer_id}:7326"
      )
      signing_key
    end

    def signed_get(peer_id, signing_key, path)
      ts = Time.now.utc.iso8601
      sig = Recollect::Sync::Crypto.sign(private_key: signing_key.to_bytes, peer_id: peer_id, timestamp: ts, body: "")
      get(path, {}, "HTTP_X_PEER_ID" => peer_id, "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig)
    end

    def signed_post(peer_id, signing_key, path, body)
      body_str = body.is_a?(String) ? body : JSON.generate(body)
      ts = Time.now.utc.iso8601
      sig = Recollect::Sync::Crypto.sign(private_key: signing_key.to_bytes, peer_id: peer_id, timestamp: ts,
        body: body_str)
      post(path, body_str, "CONTENT_TYPE" => "application/json", "HTTP_X_PEER_ID" => peer_id,
        "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig)
    end
  end
end

Minitest.after_run do
  FileUtils.rm_rf(TEST_DATA_DIR)
end
