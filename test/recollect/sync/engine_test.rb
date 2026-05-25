# frozen_string_literal: true

require "test_helper"

class Recollect::Sync::EngineTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)
    @peer_signing = Ed25519::SigningKey.generate
    @peers.add(peer_id: "peer-b", display_name: "B", public_key: @peer_signing.verify_key.to_bytes, endpoint: "http://b")
    @peers.subscribe("peer-b", "global")
  end

  def teardown
    @store.close
    super
  end

  # Fake Sync::Client: configurable manifest response + paginated pull pages.
  # Captures every POST /sync/push payload.
  def fake_client(manifest:, pull_pages:)
    pages = pull_pages.dup
    Class.new do
      define_method(:initialize) {
        @manifest = manifest
        @pages = pages
        @pushed = []
      }
      define_method(:pushed) { @pushed }
      define_method(:get) do |_path|
        Struct.new(:status, :success?, :body).new(200, true, JSON.generate(@manifest))
      end
      define_method(:post_json) do |path, payload|
        if path.start_with?("/sync/pull")
          page = @pages.shift || {records: []}
          Struct.new(:status, :success?, :body).new(200, true, JSON.generate(page))
        else
          @pushed << payload
          Struct.new(:status, :success?, :body).new(200, true, '{"accepted":1,"rejected":0}')
        end
      end
    end.new
  end

  def test_reconcile_pulls_records_from_peer
    page = {records: [{
      "global_id" => "g-from-b", "origin_peer" => "peer-b", "content" => "from b",
      "memory_type" => "note", "created_at" => "2026-05-15T00:00:00.000Z",
      "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil
    }]}
    client = fake_client(manifest: {watermarks: {}}, pull_pages: [page, {records: []}])
    engine = Recollect::Sync::Engine.new(
      store: @store, db_manager: Recollect::HTTPServer.db_manager,
      client_factory: ->(*) { client }
    )
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    db = Recollect::HTTPServer.db_manager.get_database(nil)
    row = db.instance_variable_get(:@db).get_first_row("SELECT content FROM memories WHERE global_id = ?", "g-from-b")

    assert_equal "from b", row["content"]
  end

  def test_reconcile_pushes_what_peer_is_missing
    # Local has a self-origin row; peer's manifest has no watermark for us.
    identity = Recollect::HTTPServer.local_identity
    db = Recollect::HTTPServer.db_manager.get_database(nil)
    db.store(content: "self-row", memory_type: "note", origin_peer: identity.peer_id)

    client = fake_client(manifest: {watermarks: {}}, pull_pages: [{records: []}])
    engine = Recollect::Sync::Engine.new(
      store: @store, db_manager: Recollect::HTTPServer.db_manager,
      client_factory: ->(*) { client }
    )
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    refute_empty client.pushed
    pushed_record = client.pushed.first[:records].first

    assert_equal "self-row", pushed_record["content"]
  end

  def test_reconcile_advances_watermark_after_pull
    page = {records: [{
      "global_id" => "g-wm-engine", "origin_peer" => "peer-b", "content" => "x",
      "memory_type" => "note", "created_at" => "2026-05-15T00:00:00.000Z",
      "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil
    }]}
    client = fake_client(manifest: {watermarks: {}}, pull_pages: [page, {records: []}])
    engine = Recollect::Sync::Engine.new(
      store: @store, db_manager: Recollect::HTTPServer.db_manager,
      client_factory: ->(*) { client }
    )
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    wm = Recollect::Sync::Watermarks.new(@store).get(db_name: "global")

    assert_equal "2026-05-15T00:00:00.000Z", wm["peer-b"]["created_at"]
    assert_equal "g-wm-engine", wm["peer-b"]["global_id"]
  end

  def test_reconcile_skips_push_when_peer_manifest_is_caught_up
    identity = Recollect::HTTPServer.local_identity
    db = Recollect::HTTPServer.db_manager.get_database(nil)
    db.store(content: "old", memory_type: "note", origin_peer: identity.peer_id)

    # Peer says they have our row already (manifest cursor matches our max).
    max = db.max_origin_cursor(identity.peer_id)
    client = fake_client(manifest: {watermarks: {identity.peer_id => max}}, pull_pages: [{records: []}])
    engine = Recollect::Sync::Engine.new(
      store: @store, db_manager: Recollect::HTTPServer.db_manager,
      client_factory: ->(*) { client }
    )
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    assert_empty client.pushed, "peer already has our rows; nothing to push"
  end

  def test_reconcile_records_error_on_failure
    failing_client = Object.new
    failing_client.define_singleton_method(:get) { |*| raise "boom" }
    failing_client.define_singleton_method(:post_json) { |*, **| raise "boom" }

    engine = Recollect::Sync::Engine.new(
      store: @store, db_manager: Recollect::HTTPServer.db_manager,
      client_factory: ->(*) { failing_client }
    )
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    peer = @peers.find("peer-b")

    refute_nil peer[:last_sync_error]
    assert_match(/boom/, peer[:last_sync_error])
  end
end
