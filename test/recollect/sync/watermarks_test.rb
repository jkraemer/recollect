# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::WatermarksTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @wm = Recollect::Sync::Watermarks.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def test_get_returns_empty_hash_when_no_rows
    assert_empty(@wm.get(db_name: "global"))
  end

  def test_advance_inserts_when_missing
    @wm.advance(peer_id: "peer-a", db_name: "global",
      created_at: "2026-05-01T00:00:00.000Z", global_id: "g-aaa")
    expected = {"peer-a" => {"created_at" => "2026-05-01T00:00:00.000Z", "global_id" => "g-aaa"}}

    assert_equal expected, @wm.get(db_name: "global")
  end

  def test_advance_takes_max_of_old_and_new_by_composite_cursor
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-02T00:00:00.000Z", global_id: "g-bbb")
    # Strictly earlier ts: must NOT advance.
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-zzz")

    assert_equal({"peer-a" => {"created_at" => "2026-05-02T00:00:00.000Z", "global_id" => "g-bbb"}}, @wm.get(db_name: "global"))
    # Strictly later ts: advances.
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-03T00:00:00.000Z", global_id: "g-ccc")

    assert_equal({"peer-a" => {"created_at" => "2026-05-03T00:00:00.000Z", "global_id" => "g-ccc"}}, @wm.get(db_name: "global"))
  end

  def test_advance_uses_global_id_tiebreaker_when_created_at_equal
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-mmm")
    # Same ts, lex-smaller global_id: must NOT advance.
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-aaa")

    assert_equal "g-mmm", @wm.get(db_name: "global")["peer-a"]["global_id"]
    # Same ts, lex-larger global_id: advances.
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-zzz")

    assert_equal "g-zzz", @wm.get(db_name: "global")["peer-a"]["global_id"]
  end

  def test_db_name_is_scoped
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-1")
    @wm.advance(peer_id: "peer-a", db_name: "personal", created_at: "2026-05-02T00:00:00.000Z", global_id: "g-2")

    assert_equal({"peer-a" => {"created_at" => "2026-05-01T00:00:00.000Z", "global_id" => "g-1"}}, @wm.get(db_name: "global"))
    assert_equal({"peer-a" => {"created_at" => "2026-05-02T00:00:00.000Z", "global_id" => "g-2"}}, @wm.get(db_name: "personal"))
  end

  def test_get_includes_all_peers_for_db
    @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-a")
    @wm.advance(peer_id: "peer-b", db_name: "global", created_at: "2026-05-02T00:00:00.000Z", global_id: "g-b")
    result = @wm.get(db_name: "global")

    assert_equal 2, result.size
    assert_equal "2026-05-01T00:00:00.000Z", result["peer-a"]["created_at"]
    assert_equal "2026-05-02T00:00:00.000Z", result["peer-b"]["created_at"]
  end

  def test_advance_raises_on_nil_global_id
    assert_raises(ArgumentError) do
      @wm.advance(peer_id: "peer-a", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: nil)
    end
  end

  def test_advance_raises_on_nil_created_at
    assert_raises(ArgumentError) do
      @wm.advance(peer_id: "peer-a", db_name: "global", created_at: nil, global_id: "g-x")
    end
  end
end
