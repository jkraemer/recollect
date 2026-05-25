# frozen_string_literal: true

require_relative "two_peer_helper"

class SyncTwoPeerTest < Minitest::Test
  include TwoPeerHelper

  def setup
    # Disable background sync threads (push_queue/sync_engine auto-start) — the
    # tests instantiate Sync::Engine manually and call reconcile directly.
    ENV["RECOLLECT_SYNC_DISABLE"] = "1"
    @a = make_peer(:a)
    @b = make_peer(:b)
    pair!(from: @a, to: @b)
  end

  def teardown
    teardown_peer(@a) if @a
    teardown_peer(@b) if @b
    ENV.delete("RECOLLECT_SYNC_DISABLE")
  end

  def test_pull_propagates_record_from_a_to_b
    @a.store(content: "hello from a", project: nil)

    reconcile_b_from_a

    contents = @b.list_global.map { |m| m["content"] }

    assert_includes contents, "hello from a"
  end

  def test_tombstone_propagates
    memory = @a.store(content: "doomed", project: nil)
    @a.delete(memory["id"], project: nil)

    reconcile_b_from_a

    contents = @b.list_global.map { |m| m["content"] }

    refute_includes contents, "doomed"
  end

  def test_subscription_filter_blocks_unsubscribed_db
    @a.with_config do
      Recollect::Sync::Peers.new(@a.app_class.sync_store).unsubscribe(@b.identity.peer_id, "global")
    end
    @a.store(content: "secret", project: nil)

    reconcile_b_from_a

    contents = @b.list_global.map { |m| m["content"] }

    refute_includes contents, "secret"
  end

  def test_push_on_write_propagates_a_to_b
    # Enable sync so A's push_queue starts (and short-circuits don't fire on
    # subsequent calls). HTTPServer.push_queue re-checks sync_disabled? on every
    # call, so the flag must stay unset for the duration of this test.
    ENV["RECOLLECT_SYNC_DISABLE"] = nil
    @a.with_config { @a.app_class.reset_db_manager! }

    # Touch A's push_queue to ensure it's running, then patch its client_factory.
    factory = cross_peer_client_factory(from: @a, to: @b)
    @a.with_config do
      queue = @a.app_class.push_queue

      refute_nil queue, "A's push_queue must be running for push-on-write"
      queue.instance_variable_set(:@client_factory, factory)
    end

    memory = @a.store(content: "pushed", project: nil)
    @a.with_config { @a.app_class.push_queue.flush(timeout: 5) }

    contents = @b.list_global.map { |m| m["content"] }

    assert_includes contents, "pushed"
    refute_nil memory, "store should have returned the new memory"
  end

  def test_pull_paginates_across_many_records
    identity = @a.identity
    db = @a.with_config { @a.app_class.db_manager.get_database(nil) }
    # Insert 600 distinct rows directly via Database#store (faster than going
    # through MemoriesService; we're testing pagination, not the create pipeline).
    600.times do |i|
      db.store(content: "item #{i}", memory_type: "note", origin_peer: identity.peer_id)
    end

    factory = cross_peer_client_factory(from: @b, to: @a)
    @b.with_config do
      engine = Recollect::Sync::Engine.new(
        store: @b.app_class.sync_store, db_manager: @b.app_class.db_manager,
        client_factory: factory, heartbeat_seconds: 0
      )
      a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
      engine.reconcile(peer: a_peer, db_name: "global")
    end

    count = @b.with_config { @b.app_class.db_manager.get_database(nil).count }

    assert_equal 600, count
  end

  private

  # Have B pull/push against A for the "global" db.
  def reconcile_b_from_a
    factory = cross_peer_client_factory(from: @b, to: @a)
    @b.with_config do
      engine = Recollect::Sync::Engine.new(
        store: @b.app_class.sync_store,
        db_manager: @b.app_class.db_manager,
        client_factory: factory,
        heartbeat_seconds: 0
      )
      a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
      engine.reconcile(peer: a_peer, db_name: "global")
    end
  end
end
