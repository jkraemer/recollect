# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::PairingCodesTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @codes = Recollect::Sync::PairingCodes.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def test_generate_returns_code_and_expiry
    result = @codes.generate(ttl_seconds: 300)

    assert_match(/\A[A-Z0-9]{4}-[A-Z0-9]{4}\z/, result[:code])
    assert_kind_of Time, result[:expires_at]
    assert_in_delta 300, (result[:expires_at] - Time.now.utc), 5
  end

  def test_consume_returns_true_for_valid_code
    code = @codes.generate[:code]

    assert @codes.consume(code, used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_unknown_code
    refute @codes.consume("ZZZZ-ZZZZ", used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_expired_code
    code = @codes.generate(ttl_seconds: -1)[:code] # already expired

    refute @codes.consume(code, used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_already_used_code
    code = @codes.generate[:code]

    assert @codes.consume(code, used_by_peer: "peer-x")
    refute @codes.consume(code, used_by_peer: "peer-y")
  end
end
