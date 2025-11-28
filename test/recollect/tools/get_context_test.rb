# frozen_string_literal: true

require "test_helper"

class GetContextTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)

    # Seed project data
    db = @db_manager.get_database("context-project")
    db.store(content: "Decision 1", memory_type: "decision")
    db.store(content: "Pattern 1", memory_type: "pattern")
    db.store(content: "Note 1", memory_type: "note")
    db.store(content: "Note 2", memory_type: "note")
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_returns_project_context
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])

    assert_equal "context-project", response_data["project"]
    assert_equal 4, response_data["total_memories"]
  end

  def test_returns_memories_by_type
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])
    by_type = response_data["by_type"]

    assert_equal 2, by_type["note"]
    assert_equal 1, by_type["decision"]
    assert_equal 1, by_type["pattern"]
  end

  def test_returns_recent_memories
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_predicate response_data["recent_count"], :positive?
    assert_kind_of Array, response_data["recent_memories"]
  end
end
