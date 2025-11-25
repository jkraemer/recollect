# frozen_string_literal: true

require 'test_helper'

class SearchMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new

    # Seed some data
    db = @db_manager.get_database(nil)
    db.store(content: 'Ruby threading patterns', memory_type: 'pattern')
    db.store(content: 'Python async patterns', memory_type: 'note')
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_searches_memories
    result = Recollect::Tools::SearchMemory.call(
      query: 'patterns',
      server_context: { db_manager: @db_manager }
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])
    assert_equal 2, response_data['count']
    assert_equal 'patterns', response_data['query']
  end

  def test_filters_by_memory_type
    result = Recollect::Tools::SearchMemory.call(
      query: 'patterns',
      memory_type: 'pattern',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert_equal 1, response_data['count']
    assert_includes response_data['results'].first['content'], 'Ruby'
  end

  def test_limits_results
    db = @db_manager.get_database(nil)
    5.times { |i| db.store(content: "Memory #{i} about testing") }

    result = Recollect::Tools::SearchMemory.call(
      query: 'testing',
      limit: 2,
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert_equal 2, response_data['count']
  end

  def test_searches_specific_project
    project_db = @db_manager.get_database('search-project')
    project_db.store(content: 'Project specific patterns')

    result = Recollect::Tools::SearchMemory.call(
      query: 'patterns',
      project: 'search-project',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert_equal 1, response_data['count']
    assert_equal 'search-project', response_data['results'].first['project']
  end
end
