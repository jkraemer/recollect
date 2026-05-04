# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::IdentityTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
  end

  def teardown
    @store.close
    super
  end

  def test_first_call_generates_keypair
    identity = Recollect::Sync::Identity.ensure!(@store, display_name: "test-host")

    assert_match(/\A[1-9A-HJ-NP-Za-km-z]+\z/, identity.peer_id, "peer_id should be base58")
    refute_nil identity.public_key
    refute_nil identity.private_key
    assert_equal "test-host", identity.display_name
  end

  def test_second_call_returns_same_identity
    first = Recollect::Sync::Identity.ensure!(@store, display_name: "test-host")
    second = Recollect::Sync::Identity.ensure!(@store, display_name: "ignored")

    assert_equal first.peer_id, second.peer_id
    assert_equal first.public_key, second.public_key
    assert_equal "test-host", second.display_name, "display_name from first call wins"
  end

  def test_peer_id_derivation_is_deterministic
    pub = "\x00" * 32
    expected = Recollect::Sync::Identity.derive_peer_id(pub)

    assert_equal expected, Recollect::Sync::Identity.derive_peer_id(pub)
  end

  def test_can_sign_and_verify_with_keypair
    identity = Recollect::Sync::Identity.ensure!(@store)
    signing_key = Ed25519::SigningKey.new(identity.private_key)
    verify_key = Ed25519::VerifyKey.new(identity.public_key)
    sig = signing_key.sign("hello")

    assert verify_key.verify(sig, "hello")
  end
end
