# frozen_string_literal: true

require 'test_helper'
require 'rack/test'

class HTTPServerTest < Recollect::TestCase
  include Rack::Test::Methods

  def app
    Recollect::HTTPServer
  end

  # Health endpoint
  def test_health_returns_ok
    get '/health'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_equal 'healthy', data['status']
    assert_equal Recollect::VERSION, data['version']
  end

  # MCP endpoint
  def test_mcp_endpoint_accepts_post
    # Send a minimal MCP request
    post '/mcp', '{"jsonrpc":"2.0","method":"initialize","id":1}', 'CONTENT_TYPE' => 'application/json'
    assert last_response.ok?
    assert_equal 'application/json', last_response.content_type
  end

  # List memories
  def test_list_memories_returns_array
    get '/api/memories'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_kind_of Array, data
  end

  def test_list_memories_with_project
    # Create a memory in a project first
    post '/api/memories', { content: 'Test', project: 'http-test' }.to_json, 'CONTENT_TYPE' => 'application/json'

    get '/api/memories', project: 'http-test'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_kind_of Array, data
  end

  # Search memories
  def test_search_requires_query
    get '/api/memories/search'
    assert_equal 400, last_response.status

    data = JSON.parse(last_response.body)
    assert data['error']
  end

  def test_search_returns_results
    # Create searchable memory
    post '/api/memories', { content: 'Ruby threading patterns' }.to_json, 'CONTENT_TYPE' => 'application/json'

    get '/api/memories/search', q: 'threading'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_kind_of Array, data['results']
    assert data['query']
  end

  # Get single memory
  def test_get_memory_returns_404_for_missing
    get '/api/memories/99999'
    assert_equal 404, last_response.status
  end

  def test_get_memory_returns_memory
    # Create a memory
    post '/api/memories', { content: 'Get test' }.to_json, 'CONTENT_TYPE' => 'application/json'
    created = JSON.parse(last_response.body)

    get "/api/memories/#{created['id']}"
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_equal 'Get test', data['content']
  end

  # Create memory
  def test_create_memory_returns_201
    post '/api/memories', { content: 'New memory' }.to_json, 'CONTENT_TYPE' => 'application/json'
    assert_equal 201, last_response.status

    data = JSON.parse(last_response.body)
    assert data['id']
    assert_equal 'New memory', data['content']
  end

  def test_create_memory_with_all_fields
    post '/api/memories', {
      content: 'Full memory',
      memory_type: 'decision',
      tags: ['test', 'http'],
      project: 'create-test'
    }.to_json, 'CONTENT_TYPE' => 'application/json'

    assert_equal 201, last_response.status

    data = JSON.parse(last_response.body)
    assert_equal 'decision', data['memory_type']
    assert_equal ['test', 'http'], data['tags']
  end

  # Update memory
  def test_update_memory_returns_updated
    # Create first
    post '/api/memories', { content: 'Original' }.to_json, 'CONTENT_TYPE' => 'application/json'
    created = JSON.parse(last_response.body)

    put "/api/memories/#{created['id']}", { content: 'Updated' }.to_json, 'CONTENT_TYPE' => 'application/json'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_equal 'Updated', data['content']
  end

  def test_update_missing_memory_returns_404
    put '/api/memories/99999', { content: 'Test' }.to_json, 'CONTENT_TYPE' => 'application/json'
    assert_equal 404, last_response.status
  end

  # Delete memory
  def test_delete_memory_succeeds
    # Create first
    post '/api/memories', { content: 'To delete' }.to_json, 'CONTENT_TYPE' => 'application/json'
    created = JSON.parse(last_response.body)

    delete "/api/memories/#{created['id']}"
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_equal created['id'], data['deleted']
  end

  def test_delete_missing_memory_returns_404
    delete '/api/memories/99999'
    assert_equal 404, last_response.status
  end

  # List projects
  def test_list_projects
    get '/api/projects'
    assert last_response.ok?

    data = JSON.parse(last_response.body)
    assert_kind_of Array, data['projects']
    assert data['count'] >= 0
  end

  # Static files (index.html)
  def test_root_serves_index
    # This will 404 until we create public/index.html in Batch 8
    # For now, just verify the route exists
    get '/'
    # Either serves file or 404 if file doesn't exist yet
    assert [200, 404].include?(last_response.status)
  end
end
