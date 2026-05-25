# frozen_string_literal: true

require "test_helper"
require "ed25519"
require "faraday/rack"

class Recollect::Sync::ClientTest < Minitest::Test
  def setup
    @signing_key = Ed25519::SigningKey.generate
    @public_key = @signing_key.verify_key.to_bytes
    @private_key = @signing_key.to_bytes
    @captured = nil

    # Build a fake Rack app that captures the incoming request
    @app = ->(env) {
      req = Rack::Request.new(env)
      body = env["rack.input"].read
      @captured = {
        path: req.path_info,
        method: req.request_method,
        body: body,
        headers: env.select { |k, _| k.start_with?("HTTP_X_") }
      }
      [200, {"Content-Type" => "application/json"}, ['{"ok":true}']]
    }
  end

  def test_signs_outbound_post
    client = Recollect::Sync::Client.new(
      peer_id: "local-peer", private_key: @private_key,
      endpoint: "http://peer.test", adapter: [:rack, @app]
    )
    client.post_json("/sync/push?db=global", {records: []})

    assert_equal "POST", @captured[:method]
    assert_equal "local-peer", @captured[:headers]["HTTP_X_PEER_ID"]
    refute_nil @captured[:headers]["HTTP_X_TIMESTAMP"]
    refute_nil @captured[:headers]["HTTP_X_SIGNATURE"]

    # Verify signature against captured body
    assert Recollect::Sync::Crypto.verify(
      public_key: @public_key,
      peer_id: @captured[:headers]["HTTP_X_PEER_ID"],
      timestamp: @captured[:headers]["HTTP_X_TIMESTAMP"],
      body: @captured[:body],
      signature: @captured[:headers]["HTTP_X_SIGNATURE"]
    )
  end

  def test_get_signs_with_empty_body
    client = Recollect::Sync::Client.new(
      peer_id: "local-peer", private_key: @private_key,
      endpoint: "http://peer.test", adapter: [:rack, @app]
    )
    client.get("/sync/manifest?db=global")

    assert_equal "GET", @captured[:method]
    assert_equal "", @captured[:body]
    refute_nil @captured[:headers]["HTTP_X_SIGNATURE"]
  end
end
