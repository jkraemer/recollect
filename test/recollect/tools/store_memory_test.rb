# frozen_string_literal: true

require 'test_helper'

class StoreMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_stores_memory_in_global_database
    result = Recollect::Tools::StoreMemory.call(
      content: 'Test memory',
      server_context: { db_manager: @db_manager }
    )

    assert_kind_of MCP::Tool::Response, result

    # Parse response
    response_data = JSON.parse(result.content.first[:text])
    assert response_data['success']
    assert response_data['id'] > 0
    assert_equal 'global', response_data['stored_in']
  end

  def test_stores_memory_in_project_database
    result = Recollect::Tools::StoreMemory.call(
      content: 'Project memory',
      project: 'test-project',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert response_data['success']
    assert_includes response_data['stored_in'], 'test-project'
  end

  def test_stores_with_memory_type
    result = Recollect::Tools::StoreMemory.call(
      content: 'A decision',
      memory_type: 'decision',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert response_data['success']

    # Verify stored correctly
    db = @db_manager.get_database(nil)
    memory = db.get(response_data['id'])
    assert_equal 'decision', memory['memory_type']
  end

  def test_stores_with_tags
    result = Recollect::Tools::StoreMemory.call(
      content: 'Tagged memory',
      tags: ['ruby', 'testing'],
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])

    db = @db_manager.get_database(nil)
    memory = db.get(response_data['id'])
    assert_equal ['ruby', 'testing'], memory['tags']
  end

  def test_sets_source_to_mcp
    result = Recollect::Tools::StoreMemory.call(
      content: 'MCP memory',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])

    db = @db_manager.get_database(nil)
    memory = db.get(response_data['id'])
    assert_equal 'mcp', memory['source']
  end
end
