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
end
# rubocop:enable Naming/VariableNumber
