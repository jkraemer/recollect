# frozen_string_literal: true

require "test_helper"
require "rack/test"

class MCPIntegrationTest < Recollect::TestCase
  include Rack::Test::Methods

  def app
    Recollect::HTTPServer
  end

  # Test full MCP flow: store via MCP, retrieve via HTTP
  def test_store_via_mcp_retrieve_via_http
    # Store via MCP endpoint
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "store_memory",
        arguments: {
          content: "MCP integration test memory",
          memory_type: "note",
          tags: %w[integration test]
        }
      },
      id: 1
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)

    # MCP response should have result with content
    assert mcp_response["result"]
    result_content = JSON.parse(mcp_response["result"]["content"].first["text"])

    assert result_content["success"]
    stored_id = result_content["id"]

    # Retrieve via HTTP API
    get "/api/memories/#{stored_id}"

    assert_predicate last_response, :ok?

    memory = JSON.parse(last_response.body)

    assert_equal "MCP integration test memory", memory["content"]
    assert_equal "note", memory["memory_type"]
    assert_equal %w[integration test], memory["tags"]
  end

  # Test search via MCP
  def test_search_via_mcp
    # First store something via HTTP
    post "/api/memories", {
      content: "Searchable MCP content",
      memory_type: "decision"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # Search via MCP
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "search_memory",
        arguments: {
          query: "Searchable"
        }
      },
      id: 2
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response["result"]["content"].first["text"])

    assert_operator result_content["count"], :>=, 1
    assert(result_content["results"].any? { |r| r["content"].include?("Searchable") })
  end

  # Test project isolation
  def test_project_isolation
    # Store in project A
    post "/api/memories", {
      content: "Project A memory",
      project: "project-a"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # Store in project B
    post "/api/memories", {
      content: "Project B memory",
      project: "project-b"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # Search project A only
    get "/api/memories/search", q: "memory", project: "project-a"
    data = JSON.parse(last_response.body)

    assert_equal 1, data["count"]
    assert_equal "project-a", data["results"].first["project"]
  end

  # Test list projects includes created projects
  def test_list_projects_after_creation
    # Create a project by storing a memory
    post "/api/memories", {
      content: "Integration project memory",
      project: "integration-test-project"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # List projects via MCP
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "list_projects",
        arguments: {}
      },
      id: 3
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response["result"]["content"].first["text"])

    assert_includes result_content["projects"], "integration-test-project"
  end

  # Test delete via MCP
  def test_delete_via_mcp
    # Create via HTTP
    post "/api/memories", {content: "To be deleted via MCP"}.to_json, "CONTENT_TYPE" => "application/json"
    created = JSON.parse(last_response.body)
    memory_id = created["id"]

    # Delete via MCP
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "delete_memory",
        arguments: {id: memory_id}
      },
      id: 4
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response["result"]["content"].first["text"])

    assert result_content["success"]

    # Verify deleted via HTTP
    get "/api/memories/#{memory_id}"

    assert_equal 404, last_response.status
  end

  # Test get_context via MCP
  def test_get_context_via_mcp
    # Create some memories in a project
    post "/api/memories", {
      content: "Latest session log",
      project: "context-test-project",
      memory_type: "session"
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "A note",
      project: "context-test-project",
      memory_type: "note"
    }.to_json, "CONTENT_TYPE" => "application/json"

    post "/api/memories", {
      content: "A todo item",
      project: "context-test-project",
      memory_type: "todo"
    }.to_json, "CONTENT_TYPE" => "application/json"

    # Get context via MCP
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "get_context",
        arguments: {project: "context-test-project"}
      },
      id: 5
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response["result"]["content"].first["text"])

    assert_equal "context-test-project", result_content["project"]
    assert_equal "Latest session log", result_content["last_session"]["content"]
    assert_equal 2, result_content["recent_notes_todos"].length
  end

  # A tool that rejects its arguments has failed, not the protocol: the client
  # gets a result it can show the model and retry from, not a transport error.
  def test_invalid_tool_arguments_come_back_as_a_tool_error
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "store_memory",
        arguments: {memory_type: "bogus"}
      },
      id: 20
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)

    assert_nil mcp_response["error"]
    assert mcp_response["result"]["isError"]
    assert_match(/content/, mcp_response["result"]["content"].first["text"])
  end

  # Naming a tool that does not exist is a caller mistake, so it is a protocol
  # error - and specifically "invalid params", not a server-side crash.
  def test_unknown_tool_is_an_invalid_params_error
    mcp_request = {
      jsonrpc: "2.0",
      method: "tools/call",
      params: {
        name: "no_such_tool",
        arguments: {}
      },
      id: 21
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    mcp_response = JSON.parse(last_response.body)

    assert_equal(-32602, mcp_response["error"]["code"])
  end

  def test_prompts_list
    mcp_request = {
      jsonrpc: "2.0",
      method: "prompts/list",
      id: 10
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)
    prompts = mcp_response["result"]["prompts"]

    assert prompts.any? { |p| p["name"] == "session_log" }
  end

  def test_prompts_get_session_log
    mcp_request = {
      jsonrpc: "2.0",
      method: "prompts/get",
      params: {
        name: "session_log",
        arguments: {}
      },
      id: 11
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)
    result = mcp_response["result"]

    assert result["messages"]
    assert_equal 1, result["messages"].size
    assert_equal "user", result["messages"].first["role"]
    assert_match(/Session Log/, result["messages"].first["content"]["text"])
  end

  def test_prompts_get_session_log_with_project
    mcp_request = {
      jsonrpc: "2.0",
      method: "prompts/get",
      params: {
        name: "session_log",
        arguments: {project: "test-project"}
      },
      id: 12
    }

    post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

    assert_predicate last_response, :ok?

    mcp_response = JSON.parse(last_response.body)
    result = mcp_response["result"]

    assert_match(/"test-project"/, result["messages"].first["content"]["text"])
  end

  def test_every_tool_advertises_an_output_schema
    post "/mcp", {jsonrpc: "2.0", method: "tools/list", id: 30}.to_json,
      "CONTENT_TYPE" => "application/json"

    tools = JSON.parse(last_response.body)["result"]["tools"]

    assert_equal 5, tools.length
    tools.each do |tool|
      assert tool["outputSchema"], "#{tool["name"]} must declare an outputSchema"
    end
  end

  def test_tool_calls_return_structured_content_matching_the_text
    post "/api/memories", {content: "wire structured probe"}.to_json,
      "CONTENT_TYPE" => "application/json"

    post "/mcp", {
      jsonrpc: "2.0", method: "tools/call", id: 31,
      params: {name: "search_memory", arguments: {query: "probe"}}
    }.to_json, "CONTENT_TYPE" => "application/json"

    result = JSON.parse(last_response.body)["result"]

    assert_equal JSON.parse(result["content"].first["text"]), result["structuredContent"]
  end

  def test_resources_list_reflects_projects_created_mid_session
    post "/mcp", {jsonrpc: "2.0", method: "resources/list", id: 40}.to_json,
      "CONTENT_TYPE" => "application/json"

    uris = JSON.parse(last_response.body)["result"]["resources"].map { |r| r["uri"] }

    assert_includes uris, "recollect://project/global"
    refute_includes uris, "recollect://project/fresh"

    post "/api/memories", {content: "fresh note", project: "fresh"}.to_json,
      "CONTENT_TYPE" => "application/json"

    post "/mcp", {jsonrpc: "2.0", method: "resources/list", id: 41}.to_json,
      "CONTENT_TYPE" => "application/json"

    resources = JSON.parse(last_response.body)["result"]["resources"]
    fresh = resources.find { |r| r["uri"] == "recollect://project/fresh" }

    refute_nil fresh
    assert_equal "text/markdown", fresh["mimeType"]
  end

  def test_resources_read_returns_project_markdown
    post "/api/memories", {content: "readable note", project: "readme"}.to_json,
      "CONTENT_TYPE" => "application/json"

    post "/mcp", {
      jsonrpc: "2.0", method: "resources/read", id: 42,
      params: {uri: "recollect://project/readme"}
    }.to_json, "CONTENT_TYPE" => "application/json"

    contents = JSON.parse(last_response.body)["result"]["contents"]

    assert_equal "text/markdown", contents.first["mimeType"]
    assert_includes contents.first["text"], "readable note"
  end

  def test_resources_read_resolves_a_single_memory_via_the_template
    post "/api/memories", {content: "single memory body", project: "readme"}.to_json,
      "CONTENT_TYPE" => "application/json"
    memory_id = JSON.parse(last_response.body)["id"]

    post "/mcp", {
      jsonrpc: "2.0", method: "resources/read", id: 43,
      params: {uri: "recollect://project/readme/memory/#{memory_id}"}
    }.to_json, "CONTENT_TYPE" => "application/json"

    contents = JSON.parse(last_response.body)["result"]["contents"]

    assert_includes contents.first["text"], "single memory body"
  end

  def test_resources_read_unknown_uri_is_invalid_params_with_the_uri_in_data
    post "/mcp", {
      jsonrpc: "2.0", method: "resources/read", id: 44,
      params: {uri: "recollect://project/ghost"}
    }.to_json, "CONTENT_TYPE" => "application/json"

    error = JSON.parse(last_response.body)["error"]

    assert_equal(-32602, error["code"])
    assert_equal "recollect://project/ghost", error.dig("data", "uri")
  end

  def test_resources_templates_list_contains_the_memory_template
    post "/mcp", {jsonrpc: "2.0", method: "resources/templates/list", id: 45}.to_json,
      "CONTENT_TYPE" => "application/json"

    templates = JSON.parse(last_response.body)["result"]["resourceTemplates"]

    assert_equal ["recollect://project/{project}/memory/{id}"], templates.map { |t| t["uriTemplate"] }
  end

  # NULL tags rows exist in real databases (pre-MemoriesService writes, sync
  # ingest of peer records). Storing straight through Database - bypassing
  # MemoriesService#create's `tags || []` coercion - reproduces one, so this
  # exercises deserialize's own handling of a NULL tags column under
  # server-side result validation.
  def test_search_memory_and_get_context_survive_a_null_tags_row
    global_db = Recollect::HTTPServer.db_manager.get_database(nil)
    stored = global_db.store(content: "null tags probe", memory_type: "note", tags: nil)

    post "/mcp", {
      jsonrpc: "2.0", method: "tools/call", id: 50,
      params: {name: "search_memory", arguments: {query: "null tags probe"}}
    }.to_json, "CONTENT_TYPE" => "application/json"

    response = JSON.parse(last_response.body)

    assert_nil response["error"], "search_memory must not fail validation: #{response["error"].inspect}"
    found = response.dig("result", "structuredContent", "results").find { |r| r["id"] == stored[:id] }

    refute_nil found, "expected the null-tags memory in search_memory results"
    assert_empty found["tags"]

    post "/mcp", {
      jsonrpc: "2.0", method: "tools/call", id: 51,
      params: {name: "get_context", arguments: {}}
    }.to_json, "CONTENT_TYPE" => "application/json"

    response = JSON.parse(last_response.body)

    assert_nil response["error"], "get_context must not fail validation: #{response["error"].inspect}"
    found = response.dig("result", "structuredContent", "recent_notes_todos").find { |r| r["id"] == stored[:id] }

    refute_nil found, "expected the null-tags memory in get_context recent_notes_todos"
    assert_empty found["tags"]
  end

  def test_get_context_both_shapes_pass_result_validation
    post "/api/memories", {content: "shape probe", project: "shapes"}.to_json,
      "CONTENT_TYPE" => "application/json"

    [{project: "shapes"}, {}].each do |arguments|
      post "/mcp", {
        jsonrpc: "2.0", method: "tools/call", id: 32,
        params: {name: "get_context", arguments: arguments}
      }.to_json, "CONTENT_TYPE" => "application/json"

      response = JSON.parse(last_response.body)

      assert_nil response["error"], "get_context #{arguments} must not fail validation: #{response["error"].inspect}"
      assert response.dig("result", "structuredContent")
    end
  end
end
