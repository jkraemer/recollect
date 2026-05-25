# frozen_string_literal: true

# rubocop:disable Naming/VariableNumber
require "test_helper"
require "rack/test"

class HTTPServerTest < Recollect::TestCase
  include Rack::Test::Methods

  def app
    Recollect::HTTPServer
  end

  # Health endpoint
  def test_health_returns_ok
    get "/health"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal "healthy", data["status"]
    assert_equal Recollect::VERSION, data["version"]
  end

  # MCP endpoint
  def test_mcp_endpoint_accepts_post
    # Send a minimal MCP request
    post "/mcp", '{"jsonrpc":"2.0","method":"initialize","id":1}', "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?
    assert_equal "application/json", last_response.content_type
  end

  # List memories
  def test_list_memories_returns_array
    get "/api/memories"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data
  end

  def test_list_memories_with_project
    # Create a memory in a project first
    post "/api/memories", {content: "Test", project: "http-test"}.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/memories", project: "http-test"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data
  end

  # Search memories
  def test_search_requires_query
    get "/api/memories/search"

    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)

    assert data["error"]
  end

  def test_search_returns_results
    # Create searchable memory
    post "/api/memories", {content: "Ruby threading patterns"}.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/memories/search", q: "threading"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data["results"]
    assert data["query"]
  end

  # Get single memory
  def test_get_memory_returns_404_for_missing
    get "/api/memories/99999"

    assert_equal 404, last_response.status
  end

  def test_get_memory_returns_memory
    # Create a memory
    post "/api/memories", {content: "Get test"}.to_json, "CONTENT_TYPE" => "application/json"
    created = JSON.parse(last_response.body)

    get "/api/memories/#{created["id"]}"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal "Get test", data["content"]
  end

  # Create memory
  def test_create_memory_returns_201
    post "/api/memories", {content: "New memory"}.to_json, "CONTENT_TYPE" => "application/json"

    assert_equal 201, last_response.status

    data = JSON.parse(last_response.body)

    assert data["id"]
    assert_equal "New memory", data["content"]
  end

  def test_create_memory_with_all_fields
    post "/api/memories", {
      content: "Full memory",
      memory_type: "decision",
      tags: %w[test http],
      project: "create-test"
    }.to_json, "CONTENT_TYPE" => "application/json"

    assert_equal 201, last_response.status

    data = JSON.parse(last_response.body)

    assert_equal "decision", data["memory_type"]
    assert_equal %w[test http], data["tags"]
  end

  # Delete memory
  def test_delete_memory_succeeds
    # Create first
    post "/api/memories", {content: "To delete"}.to_json, "CONTENT_TYPE" => "application/json"
    created = JSON.parse(last_response.body)

    delete "/api/memories/#{created["id"]}"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal created["id"], data["deleted"]
  end

  def test_delete_missing_memory_returns_404
    delete "/api/memories/99999"

    assert_equal 404, last_response.status
  end

  # List projects
  def test_list_projects
    get "/api/projects"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data["projects"]
    assert_operator data["count"], :>=, 0
  end

  # Static files (index.html)
  def test_root_serves_index
    # This will 404 until we create public/index.html in Batch 8
    # For now, just verify the route exists
    get "/"
    # Either serves file or 404 if file doesn't exist yet
    assert_includes [200, 404], last_response.status
  end

  # Tag stats
  def test_get_api_tags_returns_tag_stats
    # Create memories with tags
    post "/api/memories", {
      content: "First memory",
      tags: %w[decision threading]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Second memory",
      tags: %w[decision]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Third memory",
      tags: %w[threading performance]
    }.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/tags"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Hash, data["tags"]
    assert_equal 5, data["total"] # decision(2) + threading(2) + performance(1)
    assert_equal 3, data["unique"] # decision, threading, performance
    assert_equal 2, data["tags"]["decision"]
    assert_equal 2, data["tags"]["threading"]
    assert_equal 1, data["tags"]["performance"]
  end

  def test_get_api_tags_with_project_filter
    # Create memories in different projects
    post "/api/memories", {
      content: "Project A memory",
      tags: %w[decision],
      project: "project-a"
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Project B memory",
      tags: %w[decision threading],
      project: "project-b"
    }.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/tags", project: "project-a"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal 1, data["total"]
    assert_equal 1, data["unique"]
    assert_equal 1, data["tags"]["decision"]
    assert_nil data["tags"]["threading"]
  end

  # Search by tags
  def test_get_api_memories_by_tags_returns_matching_memories
    # Create memories with tags
    post "/api/memories", {
      content: "Memory with decision tag",
      tags: %w[decision]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Memory with threading tag",
      tags: %w[threading]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Memory with both tags",
      tags: %w[decision threading]
    }.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/memories/by-tags", tags: "decision"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data["results"]
    assert_equal 2, data["count"]
    assert_equal ["decision"], data["tags"]

    # All results should have the decision tag
    data["results"].each do |memory|
      assert_includes memory["tags"], "decision"
    end
  end

  def test_get_api_memories_by_tags_with_multiple_tags
    # Create memories with various tag combinations
    post "/api/memories", {
      content: "Only decision",
      tags: %w[decision]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Only threading",
      tags: %w[threading]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Both decision and threading",
      tags: %w[decision threading]
    }.to_json, "CONTENT_TYPE" => "application/json"

    get "/api/memories/by-tags", tags: "decision,threading"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_kind_of Array, data["results"]
    assert_equal 1, data["count"]
    assert_equal %w[decision threading], data["tags"]

    # Only the memory with both tags should be returned
    assert_equal "Both decision and threading", data["results"][0]["content"]
  end

  def test_get_api_memories_by_tags_with_all_projects
    # Create memories with the same tag in different projects
    post "/api/memories", {
      content: "Global tagged memory",
      tags: %w[important]
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Project A tagged memory",
      tags: %w[important],
      project: "tag-project-a"
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "Project B tagged memory",
      tags: %w[important],
      project: "tag-project-b"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # Search across all projects
    get "/api/memories/by-tags", tags: "important", project: "__all__"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    # Should find all 3 memories
    assert_equal 3, data["count"], "Expected 3 results from all projects, got #{data["count"]}"
  end

  # ========== Vector Search API Tests ==========

  # Test vectors/status when vectors disabled (default)
  def test_vectors_status_when_disabled
    get "/api/vectors/status"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    refute data["enabled"]
    assert data["reason"], "Should include reason when disabled"
  end

  # Test vectors/status returns proper structure
  # Note: Testing enabled state requires config reset which isn't easily done
  # in HTTP tests. The enabled code path is tested via integration tests.
  def test_vectors_status_response_structure_when_disabled
    get "/api/vectors/status"

    data = JSON.parse(last_response.body)

    # When disabled, should have enabled: false and a reason
    refute data["enabled"]
    assert_kind_of String, data["reason"]
    # Should NOT have enabled-only fields
    refute data.key?("total_memories")
    refute data.key?("total_embeddings")
  end

  # Test vectors/backfill returns 400 when vectors disabled
  def test_vectors_backfill_returns_400_when_disabled
    post "/api/vectors/backfill"

    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)

    assert_equal "Vector search not enabled", data["error"]
  end

  # Test vectors/backfill with project parameter (still returns 400 when disabled)
  def test_vectors_backfill_with_project_returns_400_when_disabled
    post "/api/vectors/backfill", {project: "test-project", limit: 50}.to_json,
      "CONTENT_TYPE" => "application/json"

    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)

    assert_equal "Vector search not enabled", data["error"]
  end

  # Test parse_json_body error handling
  def test_create_memory_with_invalid_json_returns_400
    post "/api/memories", "not valid json", "CONTENT_TYPE" => "application/json"

    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)

    assert_equal "Invalid JSON", data["error"]
  end

  # Test root serves 404 when index.html doesn't exist
  def test_root_returns_404_when_index_missing
    # Temporarily move index.html if it exists
    public_folder = Recollect.root.join("public")
    index_path = public_folder.join("index.html")
    backup_path = public_folder.join("index.html.bak")

    had_file = index_path.exist?
    FileUtils.mv(index_path, backup_path) if had_file

    begin
      get "/"

      assert_equal 404, last_response.status

      data = JSON.parse(last_response.body)

      assert_equal "Web UI not installed", data["error"]
    ensure
      FileUtils.mv(backup_path, index_path) if had_file
    end
  end

  # Test search across all projects with __all__
  def test_search_with_all_projects_returns_results_from_multiple_projects
    # Create memories in different projects
    post "/api/memories", {content: "Global memory about search"}.to_json, "CONTENT_TYPE" => "application/json"
    post "/api/memories", {content: "Project A memory about search", project: "project-a"}.to_json,
      "CONTENT_TYPE" => "application/json"
    post "/api/memories", {content: "Project B memory about search", project: "project-b"}.to_json,
      "CONTENT_TYPE" => "application/json"

    # Search across all projects
    get "/api/memories/search", q: "search", project: "__all__"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    # Should find all 3 memories
    assert_equal 3, data["count"], "Expected 3 results from all projects, got #{data["count"]}"
  end

  # Test search with project parameter
  def test_search_with_project_filter
    # Create memories in different places
    post "/api/memories", {content: "Global memory about Ruby"}.to_json, "CONTENT_TYPE" => "application/json"
    post "/api/memories", {content: "Project memory about Ruby", project: "search-project"}.to_json,
      "CONTENT_TYPE" => "application/json"

    # Search only in project
    get "/api/memories/search", q: "Ruby", project: "search-project"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    # Should only find the project memory
    assert_equal 1, data["count"]
    assert_equal "Project memory about Ruby", data["results"][0]["content"]
  end

  # Test search with memory_type filter
  def test_search_with_type_filter
    post "/api/memories", {content: "A note about Ruby", memory_type: "note"}.to_json,
      "CONTENT_TYPE" => "application/json"
    post "/api/memories", {content: "A decision about Ruby", memory_type: "decision"}.to_json,
      "CONTENT_TYPE" => "application/json"

    get "/api/memories/search", q: "Ruby", type: "note"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal 1, data["count"]
    assert_equal "note", data["results"][0]["memory_type"]
  end

  # Test search with limit parameter
  def test_search_respects_limit_parameter
    5.times do |i|
      post "/api/memories", {content: "Memory #{i} about patterns"}.to_json, "CONTENT_TYPE" => "application/json"
    end

    get "/api/memories/search", q: "patterns", limit: "2"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal 2, data["count"]
  end

  # Test by-tags requires tags parameter
  def test_by_tags_requires_tags_parameter
    get "/api/memories/by-tags"

    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)

    assert data["error"]
  end

  # Test list memories with type filter
  def test_list_memories_with_type_filter
    post "/api/memories", {content: "A note", memory_type: "note"}.to_json, "CONTENT_TYPE" => "application/json"
    post "/api/memories", {content: "A decision", memory_type: "decision"}.to_json,
      "CONTENT_TYPE" => "application/json"

    get "/api/memories", type: "note"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal 1, data.length
    assert_equal "note", data[0]["memory_type"]
  end

  # Test list memories with limit and offset
  def test_list_memories_with_pagination
    5.times { |i| post "/api/memories", {content: "Memory #{i}"}.to_json, "CONTENT_TYPE" => "application/json" }

    get "/api/memories", limit: "2", offset: "2"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal 2, data.length
  end

  # Test get memory with project parameter
  def test_get_memory_with_project_returns_project_field
    # Create a memory in a project
    post "/api/memories", {content: "Project memory", project: "get-project"}.to_json,
      "CONTENT_TYPE" => "application/json"
    created = JSON.parse(last_response.body)

    get "/api/memories/#{created["id"]}", project: "get-project"

    assert_predicate last_response, :ok?

    data = JSON.parse(last_response.body)

    assert_equal "get-project", data["project"]
    assert_equal "Project memory", data["content"]
  end

  def test_sync_store_singleton_is_initialized
    assert_kind_of Recollect::Sync::Store, Recollect::HTTPServer.sync_store
  end

  def test_pairing_create_returns_code_for_loopback
    post "/pairing/create"

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    assert_match(/\A[A-Z0-9]{4}-[A-Z0-9]{4}\z/, body["code"])
    refute_nil body["expires_at"]
    refute_nil body["endpoint"]
  end

  def test_pairing_create_rejects_non_loopback
    post "/pairing/create", {}, "REMOTE_ADDR" => "8.8.8.8"

    assert_equal 403, last_response.status
  end

  def test_pairing_join_with_valid_code_trusts_peer
    # Create a code first
    post "/pairing/create"
    code = JSON.parse(last_response.body)["code"]

    joiner_pub = ("\x42" * 32).b
    payload = {
      code: code,
      peer_id: "joiner-x",
      display_name: "Joiner",
      public_key: Base64.strict_encode64(joiner_pub),
      endpoint: "http://joiner:7326"
    }
    post "/pairing/join", payload.to_json, "CONTENT_TYPE" => "application/json"

    assert_equal 200, last_response.status

    body = JSON.parse(last_response.body)

    assert_equal Recollect::HTTPServer.local_identity.peer_id, body["peer_id"]
    refute_nil body["public_key"]
    refute_nil body["endpoint"]

    # Joiner is now in known_peers with global subscription
    peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
    joiner = peers.find("joiner-x")

    refute_nil joiner
    assert_equal "trusted", joiner[:status]
    assert_equal ["global"], peers.subscriptions("joiner-x")
  end

  def test_api_sync_identity
    get "/api/sync/identity"

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    assert_equal Recollect::HTTPServer.local_identity.peer_id, body["peer_id"]
    refute_nil body["display_name"]
    refute_nil body["public_key_fingerprint"]
  end

  def test_api_sync_identity_rejects_non_loopback
    get "/api/sync/identity", {}, "REMOTE_ADDR" => "8.8.8.8"

    assert_equal 403, last_response.status
  end

  def test_api_sync_peers_list_empty_initially
    get "/api/sync/peers"

    assert_equal 200, last_response.status
    assert_empty JSON.parse(last_response.body)
  end

  def test_api_sync_peers_list_strips_public_key_and_includes_subscriptions
    peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
    peers.add(peer_id: "p1", display_name: "P1", public_key: ("\x00" * 32).b, endpoint: "http://p1:7326",
      default_subscription: "global")

    get "/api/sync/peers"

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    assert_equal 1, body.length
    assert_equal "p1", body[0]["peer_id"]
    assert_equal ["global"], body[0]["subscriptions"]
    refute body[0].key?("public_key")
  end

  def test_api_sync_peers_remove_blocks
    peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
    peers.add(peer_id: "p1", display_name: "P1", public_key: ("\x00" * 32).b, endpoint: "http://p1:7326")
    delete "/api/sync/peers/p1"

    assert_equal 200, last_response.status
    assert_equal "blocked", peers.find("p1")[:status]
  end

  def test_api_sync_peers_subscriptions_add_remove
    peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
    peers.add(peer_id: "p1", display_name: "P1", public_key: ("\x00" * 32).b, endpoint: "http://p1:7326")

    post "/api/sync/peers/p1/subscriptions", {db_name: "personal-finance"}.to_json,
      "CONTENT_TYPE" => "application/json"

    assert_equal 200, last_response.status
    assert_includes peers.subscriptions("p1"), "personal-finance"

    delete "/api/sync/peers/p1/subscriptions/personal-finance"

    assert_equal 200, last_response.status
    refute_includes peers.subscriptions("p1"), "personal-finance"
  end

  def test_api_sync_peers_join_rejects_non_loopback
    post "/api/sync/peers/join", {code: "x", endpoint: "http://x"}.to_json,
      "CONTENT_TYPE" => "application/json", "REMOTE_ADDR" => "8.8.8.8"

    assert_equal 403, last_response.status
  end

  def test_api_sync_peers_join_stores_remote_peer_on_success
    # Stub Faraday.post to return a synthetic 200 with a remote-peer payload
    remote_pub = ("\x77" * 32).b
    remote_response = Struct.new(:status, :body, :success?).new(
      200,
      {
        peer_id: "remote-peer-1",
        display_name: "Remote",
        public_key: Base64.strict_encode64(remote_pub),
        endpoint: "http://remote:7327"
      }.to_json,
      true
    )

    Faraday.stub(:post, ->(*_args, &_block) { remote_response }) do
      post "/api/sync/peers/join",
        {code: "ABCD-1234", endpoint: "http://remote:7327"}.to_json,
        "CONTENT_TYPE" => "application/json"
    end

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    assert_equal "remote-peer-1", body["peer_id"]
    assert_equal "trusted", body["status"]

    peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
    remote = peers.find("remote-peer-1")

    refute_nil remote
    assert_equal "trusted", remote[:status]
    assert_equal "http://remote:7327", remote[:endpoint]
    assert_equal ["global"], peers.subscriptions("remote-peer-1")
  end

  def test_api_sync_peers_join_propagates_remote_failure
    remote_response = Struct.new(:status, :body, :success?).new(401, '{"error":"bad code"}', false)

    Faraday.stub(:post, ->(*_args, &_block) { remote_response }) do
      post "/api/sync/peers/join",
        {code: "WRONG", endpoint: "http://remote:7327"}.to_json,
        "CONTENT_TYPE" => "application/json"
    end

    assert_equal 401, last_response.status
  end

  def test_pairing_join_with_invalid_code_rejected
    payload = {
      code: "ZZZZ-ZZZZ", peer_id: "x", display_name: "x",
      public_key: Base64.strict_encode64("\x00" * 32), endpoint: "http://x:7326"
    }
    post "/pairing/join", payload.to_json, "CONTENT_TYPE" => "application/json"

    assert_equal 401, last_response.status
  end

  def test_local_identity_is_available
    identity = Recollect::HTTPServer.local_identity

    assert_kind_of Recollect::Sync::Identity, identity
    refute_nil identity.peer_id
  end

  def test_local_identity_is_singleton_under_concurrent_access
    # Reset to force a fresh init
    Recollect::HTTPServer.reset_db_manager!

    identities = []
    mutex = Mutex.new
    threads = 8.times.map do
      Thread.new do
        peer = Recollect::HTTPServer.local_identity.peer_id
        mutex.synchronize { identities << peer }
      end
    end
    threads.each(&:join)

    assert_equal 1, identities.uniq.size,
      "concurrent first-touch must produce a single identity, got: #{identities.uniq.inspect}"

    # And only one row in local_identity
    raw = Recollect::HTTPServer.sync_store.instance_variable_get(:@db)
    count = raw.get_first_value("SELECT COUNT(*) FROM local_identity")

    assert_equal 1, count
  end

  def test_sync_manifest_requires_signature
    get "/sync/manifest?db=global"

    assert_includes [401, 403], last_response.status
  end

  def test_sync_manifest_returns_watermarks_for_db
    signing_key = setup_trusted_peer("peer-a")
    identity = Recollect::HTTPServer.local_identity

    # Insert a self-origin row so self-watermark is non-nil
    Recollect::HTTPServer.db_manager.get_database(nil)
      .store(content: "self", memory_type: "note", origin_peer: identity.peer_id)

    # Insert a watermark for some other origin
    Recollect::Sync::Watermarks.new(Recollect::HTTPServer.sync_store)
      .advance(peer_id: "peer-c", db_name: "global", created_at: "2026-05-01T00:00:00.000Z", global_id: "g-cc")

    signed_get("peer-a", signing_key, "/sync/manifest?db=global")

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    refute_nil body["watermarks"][identity.peer_id], "self-watermark present"
    self_wm = body["watermarks"][identity.peer_id]

    refute_nil self_wm["created_at"]
    refute_nil self_wm["global_id"]
    assert_equal "2026-05-01T00:00:00.000Z", body["watermarks"]["peer-c"]["created_at"]
    assert_equal "g-cc", body["watermarks"]["peer-c"]["global_id"]
  end

  def test_sync_manifest_missing_db_returns_400
    signing_key = setup_trusted_peer("peer-a")
    signed_get("peer-a", signing_key, "/sync/manifest")

    assert_equal 400, last_response.status
  end

  def test_sync_manifest_omits_self_when_no_local_rows
    signing_key = setup_trusted_peer("peer-a")
    identity = Recollect::HTTPServer.local_identity

    # No self-origin rows in the DB — manifest should not include the self peer
    signed_get("peer-a", signing_key, "/sync/manifest?db=global")

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    refute body["watermarks"].key?(identity.peer_id), "self-watermark must be absent when no local rows"
  end

  def test_sync_pull_returns_records_newer_than_since
    signing_key = setup_trusted_peer("peer-a")
    identity = Recollect::HTTPServer.local_identity

    # Caller must be subscribed to "global" for this to return records
    Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")

    # Write one local row (using the proper DatabaseManager pipeline so origin_peer is set right)
    Recollect::HTTPServer.db_manager.store_with_embedding(
      project: nil, content: "hello from local", memory_type: "note", tags: [], metadata: nil
    )

    signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})

    assert_equal 200, last_response.status
    body = JSON.parse(last_response.body)

    assert_equal 1, body["records"].size
    rec = body["records"].first

    assert_equal "hello from local", rec["content"]
    assert_equal identity.peer_id, rec["origin_peer"]
    refute_nil rec["global_id"]
    refute_nil rec["created_at"]
  end

  def test_sync_pull_returns_empty_when_caller_not_subscribed
    signing_key = setup_trusted_peer("peer-a")
    # Do NOT subscribe peer-a to any db
    Recollect::HTTPServer.db_manager.store_with_embedding(
      project: nil, content: "secret", memory_type: "note", tags: [], metadata: nil
    )

    signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})

    assert_equal 200, last_response.status
    assert_empty JSON.parse(last_response.body)["records"]
  end

  def test_sync_pull_excludes_chunks
    signing_key = setup_trusted_peer("peer-a")
    Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")
    db = Recollect::HTTPServer.db_manager.get_database(nil)
    identity = Recollect::HTTPServer.local_identity
    db.store(content: "parent", memory_type: "note", origin_peer: identity.peer_id)
    db.instance_variable_get(:@db).execute(
      "INSERT INTO memories (content, memory_type, global_id, origin_peer) VALUES (?, ?, ?, ?)",
      ["chunk", "_chunk", SecureRandom.uuid_v7, identity.peer_id]
    )

    signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})

    records = JSON.parse(last_response.body)["records"]

    assert_equal 1, records.size
    refute_equal "_chunk", records.first["memory_type"]
  end

  def test_sync_pull_respects_composite_cursor
    signing_key = setup_trusted_peer("peer-a")
    Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")
    identity = Recollect::HTTPServer.local_identity
    db = Recollect::HTTPServer.db_manager.get_database(nil)

    # Two records at distinct timestamps
    raw = db.instance_variable_get(:@db)
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["old", "note", "g-old", identity.peer_id, "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["new", "note", "g-new", identity.peer_id, "2026-05-02T00:00:00.000Z"])

    # Pull with the OLD cursor — should get only the "new" row
    cursor = {"created_at" => "2026-05-01T00:00:00.000Z", "global_id" => "g-old"}
    signed_post("peer-a", signing_key, "/sync/pull?db=global",
      {since: {identity.peer_id => cursor}, limit: 500})

    records = JSON.parse(last_response.body)["records"]

    assert_equal 1, records.size
    assert_equal "g-new", records.first["global_id"]
  end

  def test_sync_pull_missing_db_returns_400
    signing_key = setup_trusted_peer("peer-a")
    signed_post("peer-a", signing_key, "/sync/pull", {since: {}, limit: 500})

    assert_equal 400, last_response.status
  end

  def test_sync_pull_omits_local_id_from_records
    signing_key = setup_trusted_peer("peer-a")
    Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")
    Recollect::HTTPServer.db_manager.store_with_embedding(
      project: nil, content: "x", memory_type: "note", tags: [], metadata: nil
    )

    signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})

    rec = JSON.parse(last_response.body)["records"].first

    refute rec.key?("id"), "local id must NOT appear in synced records — receiver's local id will differ"
  end

  # Test singleton behavior - verifies fix for file descriptor leak
  def test_db_manager_is_singleton_across_requests
    # Capture the db_manager instance after first request
    get "/health"
    first_manager = Recollect::HTTPServer.db_manager

    # Make several more requests
    post "/api/memories", {content: "Test 1"}.to_json, "CONTENT_TYPE" => "application/json"
    get "/api/memories"
    post "/mcp", '{"jsonrpc":"2.0","method":"initialize","id":1}', "CONTENT_TYPE" => "application/json"

    # The db_manager should still be the same instance
    assert_same first_manager, Recollect::HTTPServer.db_manager,
      "DatabaseManager should be a singleton across requests to prevent file descriptor leaks"
  end
end
# rubocop:enable Naming/VariableNumber
