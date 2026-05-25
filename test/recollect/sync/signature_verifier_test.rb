# frozen_string_literal: true

require "test_helper"
require "ed25519"

class Recollect::Sync::SignatureVerifierTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)

    @signing_key = Ed25519::SigningKey.generate
    @peer_public = @signing_key.verify_key.to_bytes
    @peers.add(peer_id: "peer-a", display_name: "A", public_key: @peer_public, endpoint: "http://a:7326")

    @verifier = Recollect::Sync::SignatureVerifier.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def signed_env(body, peer_id: "peer-a", ts: Time.now.utc.iso8601)
    sig = Recollect::Sync::Crypto.sign(
      private_key: @signing_key.to_bytes, peer_id: peer_id, timestamp: ts, body: body
    )
    {"HTTP_X_PEER_ID" => peer_id, "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig, "body" => body}
  end

  def test_valid_signature_returns_peer
    env = signed_env("hello")
    result = @verifier.verify(headers: env, body: env["body"])

    assert_equal :ok, result[:status]
    assert_equal "peer-a", result[:peer][:peer_id]
  end

  def test_missing_headers_returns_unauthorized
    result = @verifier.verify(headers: {}, body: "")

    assert_equal :unauthorized, result[:status]
  end

  def test_unknown_peer_returns_forbidden
    env = signed_env("x", peer_id: "ghost")
    result = @verifier.verify(headers: env, body: env["body"])

    assert_equal :forbidden, result[:status]
  end

  def test_blocked_peer_returns_forbidden
    @peers.block("peer-a")
    env = signed_env("x")
    result = @verifier.verify(headers: env, body: env["body"])

    assert_equal :forbidden, result[:status]
  end

  def test_mutated_body_returns_unauthorized
    env = signed_env("real-body")
    result = @verifier.verify(headers: env, body: "tampered")

    assert_equal :unauthorized, result[:status]
  end

  def test_stale_timestamp_returns_unauthorized
    env = signed_env("x", ts: (Time.now.utc - 600).iso8601)
    result = @verifier.verify(headers: env, body: env["body"])

    assert_equal :unauthorized, result[:status]
  end
end
