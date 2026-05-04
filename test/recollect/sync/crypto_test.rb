# frozen_string_literal: true

require "test_helper"
require "ed25519"

class Recollect::Sync::CryptoTest < Minitest::Test
  def setup
    @signing_key = Ed25519::SigningKey.generate
    @public_key = @signing_key.verify_key.to_bytes
    @private_key = @signing_key.to_bytes
    @peer_id = "peer-x"
    @ts = Time.now.utc.iso8601
    @body = '{"hello":"world"}'
  end

  def test_sign_and_verify_round_trip
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)

    assert Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: @ts, body: @body, signature: sig)
  end

  def test_verify_fails_on_mutated_body
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)

    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: @ts, body: "tampered", signature: sig)
  end

  def test_verify_fails_on_mutated_peer_id
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)

    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: "other", timestamp: @ts, body: @body, signature: sig)
  end

  def test_verify_rejects_skewed_timestamp
    old = (Time.now.utc - 3600).iso8601
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: old, body: @body)

    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: old, body: @body, signature: sig)
  end

  def test_verify_accepts_within_skew
    old = (Time.now.utc - 60).iso8601
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: old, body: @body)

    assert Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: old, body: @body, signature: sig)
  end
end
