# frozen_string_literal: true

require "test_helper"

class ListProjectsTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_returns_empty_list_initially
    result = Recollect::Tools::ListProjects.call(
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])

    assert_kind_of Array, response_data["projects"]
  end

  def test_returns_created_projects
    # Create some projects
    @db_manager.get_database("project-a").store(content: "Test")
    @db_manager.get_database("project-b").store(content: "Test")

    result = Recollect::Tools::ListProjects.call(
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_includes response_data["projects"], "project_a"
    assert_includes response_data["projects"], "project_b"
    assert_equal response_data["projects"].length, response_data["count"]
  end
end
