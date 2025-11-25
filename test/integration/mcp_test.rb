# frozen_string_literal: true

require 'test_helper'
require 'rack/test'

class MCPIntegrationTest < Recollect::TestCase
  include Rack::Test::Methods

  def app
    Recollect::HTTPServer
  end

  # Test full MCP flow: store via MCP, retrieve via HTTP
  def test_store_via_mcp_retrieve_via_http
    # Store via MCP endpoint
    mcp_request = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'store_memory',
        arguments: {
          content: 'MCP integration test memory',
          memory_type: 'note',
          tags: ['integration', 'test']
        }
      },
      id: 1
    }

    post '/mcp', mcp_request.to_json, 'CONTENT_TYPE' => 'application/json'
    assert last_response.ok?

    mcp_response = JSON.parse(last_response.body)

    # MCP response should have result with content
    assert mcp_response['result']
    result_content = JSON.parse(mcp_response['result']['content'].first['text'])
    assert result_content['success']
    stored_id = result_content['id']

    # Retrieve via HTTP API
    get "/api/memories/#{stored_id}"
    assert last_response.ok?

    memory = JSON.parse(last_response.body)
    assert_equal 'MCP integration test memory', memory['content']
    assert_equal 'note', memory['memory_type']
    assert_equal ['integration', 'test'], memory['tags']
  end

  # Test search via MCP
  def test_search_via_mcp
    # First store something via HTTP
    post '/api/memories', {
      content: 'Searchable MCP content',
      memory_type: 'decision'
    }.to_json, 'CONTENT_TYPE' => 'application/json'

    # Search via MCP
    mcp_request = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'search_memory',
        arguments: {
          query: 'Searchable'
        }
      },
      id: 2
    }

    post '/mcp', mcp_request.to_json, 'CONTENT_TYPE' => 'application/json'
    assert last_response.ok?

    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response['result']['content'].first['text'])

    assert result_content['count'] >= 1
    assert result_content['results'].any? { |r| r['content'].include?('Searchable') }
  end

  # Test project isolation
  def test_project_isolation
    # Store in project A
    post '/api/memories', {
      content: 'Project A memory',
      project: 'project-a'
    }.to_json, 'CONTENT_TYPE' => 'application/json'

    # Store in project B
    post '/api/memories', {
      content: 'Project B memory',
      project: 'project-b'
    }.to_json, 'CONTENT_TYPE' => 'application/json'

    # Search project A only
    get '/api/memories/search', q: 'memory', project: 'project-a'
    data = JSON.parse(last_response.body)

    assert_equal 1, data['count']
    assert_equal 'project-a', data['results'].first['project']
  end

  # Test list projects includes created projects
  def test_list_projects_after_creation
    # Create a project by storing a memory
    post '/api/memories', {
      content: 'Integration project memory',
      project: 'integration-test-project'
    }.to_json, 'CONTENT_TYPE' => 'application/json'

    # List projects via MCP
    mcp_request = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'list_projects',
        arguments: {}
      },
      id: 3
    }

    post '/mcp', mcp_request.to_json, 'CONTENT_TYPE' => 'application/json'
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response['result']['content'].first['text'])

    assert_includes result_content['projects'], 'integration-test-project'
  end

  # Test delete via MCP
  def test_delete_via_mcp
    # Create via HTTP
    post '/api/memories', { content: 'To be deleted via MCP' }.to_json, 'CONTENT_TYPE' => 'application/json'
    created = JSON.parse(last_response.body)
    memory_id = created['id']

    # Delete via MCP
    mcp_request = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'delete_memory',
        arguments: { id: memory_id }
      },
      id: 4
    }

    post '/mcp', mcp_request.to_json, 'CONTENT_TYPE' => 'application/json'
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response['result']['content'].first['text'])

    assert result_content['success']

    # Verify deleted via HTTP
    get "/api/memories/#{memory_id}"
    assert_equal 404, last_response.status
  end

  # Test get_context via MCP
  def test_get_context_via_mcp
    # Create some memories in a project
    3.times do |i|
      post '/api/memories', {
        content: "Context test memory #{i}",
        project: 'context-test-project',
        memory_type: i.even? ? 'note' : 'decision'
      }.to_json, 'CONTENT_TYPE' => 'application/json'
    end

    # Get context via MCP
    mcp_request = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'get_context',
        arguments: { project: 'context-test-project' }
      },
      id: 5
    }

    post '/mcp', mcp_request.to_json, 'CONTENT_TYPE' => 'application/json'
    mcp_response = JSON.parse(last_response.body)
    result_content = JSON.parse(mcp_response['result']['content'].first['text'])

    assert_equal 'context-test-project', result_content['project']
    assert_equal 3, result_content['total_memories']
    assert result_content['by_type']['note'] >= 1
    assert result_content['by_type']['decision'] >= 1
  end
end
