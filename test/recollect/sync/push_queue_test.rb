# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::PushQueueTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)

    # Add one trusted peer subscribed to global
    @peer_signing = Ed25519::SigningKey.generate
    @peers.add(peer_id: "peer-b", display_name: "B", public_key: @peer_signing.verify_key.to_bytes, endpoint: "http://b:1")
    @peers.subscribe("peer-b", "global")

    @captured_pushes = []
    @client_factory = ->(peer) {
      double = Object.new
      pushes = @captured_pushes
      double.define_singleton_method(:post_json) { |path, payload|
        pushes << {peer_id: peer[:peer_id], path: path, payload: payload}
        Struct.new(:status, :success?).new(200, true)
      }
      double
    }

    @db_manager = Recollect::HTTPServer.db_manager
    @queue = Recollect::Sync::PushQueue.new(
      store: @store, db_manager: @db_manager, client_factory: @client_factory, size: 100
    )
    @queue.start
  end

  def teardown
    @queue.stop
    @store.close
    super
  end

  def test_enqueue_fans_out_to_subscribed_peers
    db = @db_manager.get_database(nil)
    result = db.store(content: "broadcast", memory_type: "note")
    @queue.enqueue(global_id: result[:global_id], db_name: "global")
    @queue.flush

    assert_equal 1, @captured_pushes.size
    assert_equal "peer-b", @captured_pushes.first[:peer_id]
    assert_equal "/sync/push?db=global", @captured_pushes.first[:path]
    assert_equal result[:global_id], @captured_pushes.first[:payload][:records].first["global_id"]
  end

  def test_enqueue_skips_unsubscribed_dbs
    db = @db_manager.get_database("personal")
    result = db.store(content: "private", memory_type: "note")
    @queue.enqueue(global_id: result[:global_id], db_name: "personal")
    @queue.flush

    assert_empty @captured_pushes
  end
end
