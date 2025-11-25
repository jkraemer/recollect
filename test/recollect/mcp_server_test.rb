# frozen_string_literal: true

require 'test_helper'

class MCPServerTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_build_returns_mcp_server
    server = Recollect::MCPServer.build(@db_manager)
    assert_kind_of MCP::Server, server
  end

  def test_server_has_correct_name
    server = Recollect::MCPServer.build(@db_manager)
    assert_equal 'recollect', server.name
  end

  def test_server_has_version
    server = Recollect::MCPServer.build(@db_manager)
    assert_equal Recollect::VERSION, server.version
  end

  def test_server_has_all_tools
    server = Recollect::MCPServer.build(@db_manager)

    # server.tools is a Hash with tool names as keys
    tool_names = server.tools.keys

    assert_includes tool_names, 'store_memory'
    assert_includes tool_names, 'search_memory'
    assert_includes tool_names, 'get_context'
    assert_includes tool_names, 'list_projects'
    assert_includes tool_names, 'delete_memory'
  end

  def test_server_context_includes_db_manager
    server = Recollect::MCPServer.build(@db_manager)
    assert_equal @db_manager, server.server_context[:db_manager]
  end
end
