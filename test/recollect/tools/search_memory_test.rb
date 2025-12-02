# frozen_string_literal: true

require "test_helper"

class SearchMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)

    # Seed some data
    db = @db_manager.get_database(nil)
    db.store(content: "Ruby threading patterns", memory_type: "pattern")
    db.store(content: "Python async patterns", memory_type: "note")
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_searches_memories
    result = Recollect::Tools::SearchMemory.call(
      query: "patterns",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 2, response_data["count"]
    assert_equal "patterns", response_data["query"]
  end

  def test_filters_by_memory_type
    result = Recollect::Tools::SearchMemory.call(
      query: "patterns",
      memory_type: "pattern",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_includes response_data["results"].first["content"], "Ruby"
  end

  def test_limits_results
    db = @db_manager.get_database(nil)
    5.times { |i| db.store(content: "Memory #{i} about testing") }

    result = Recollect::Tools::SearchMemory.call(
      query: "testing",
      limit: 2,
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 2, response_data["count"]
  end

  def test_searches_specific_project
    project_db = @db_manager.get_database("search-project")
    project_db.store(content: "Project specific patterns")

    result = Recollect::Tools::SearchMemory.call(
      query: "patterns",
      project: "search-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_equal "search-project", response_data["results"].first["project"]
  end

  def test_filters_by_single_tag
    db = @db_manager.get_database(nil)
    db.store(content: "Architecture decision about database", tags: %w[architecture decision])
    db.store(content: "Bug fix for login", tags: %w[bug authentication])
    db.store(content: "Another architecture note", tags: %w[architecture])

    result = Recollect::Tools::SearchMemory.call(
      query: "",
      tags: ["architecture"],
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 2, response_data["count"]
    response_data["results"].each do |memory|
      assert_includes memory["tags"], "architecture"
    end
  end

  def test_filters_by_multiple_tags_with_and_logic
    db = @db_manager.get_database(nil)
    db.store(content: "Architecture decision about database", tags: %w[architecture decision])
    db.store(content: "Bug fix for login", tags: %w[bug authentication])
    db.store(content: "Another architecture note", tags: %w[architecture])

    result = Recollect::Tools::SearchMemory.call(
      query: "",
      tags: %w[architecture decision],
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_includes response_data["results"].first["tags"], "architecture"
    assert_includes response_data["results"].first["tags"], "decision"
  end

  def test_filters_by_tags_in_specific_project
    project_db = @db_manager.get_database("tag-project")
    project_db.store(content: "Project memory with tags", tags: %w[important])

    global_db = @db_manager.get_database(nil)
    global_db.store(content: "Global memory with tags", tags: %w[important])

    result = Recollect::Tools::SearchMemory.call(
      query: "",
      tags: ["important"],
      project: "tag-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_equal "tag-project", response_data["results"].first["project"]
  end

  def test_rejects_old_type_decision
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::SearchMemory.input_schema.validate_arguments(
        query: "test",
        memory_type: "decision"
      )
    end
  end

  def test_rejects_old_type_pattern
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::SearchMemory.input_schema.validate_arguments(
        query: "test",
        memory_type: "pattern"
      )
    end
  end

  def test_rejects_old_type_bug
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::SearchMemory.input_schema.validate_arguments(
        query: "test",
        memory_type: "bug"
      )
    end
  end

  def test_rejects_old_type_learning
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::SearchMemory.input_schema.validate_arguments(
        query: "test",
        memory_type: "learning"
      )
    end
  end

  def test_accepts_session_memory_type
    db = @db_manager.get_database(nil)
    db.store(content: "Session summary for feature work", memory_type: "session")
    db.store(content: "Regular note", memory_type: "note")

    result = Recollect::Tools::SearchMemory.call(
      query: "summary",
      memory_type: "session",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_equal "session", response_data["results"].first["memory_type"]
  end

  def test_filters_by_array_of_memory_types
    db = @db_manager.get_database(nil)
    db.store(content: "A session about testing", memory_type: "session")
    db.store(content: "A note about testing", memory_type: "note")
    db.store(content: "A todo about testing", memory_type: "todo")

    result = Recollect::Tools::SearchMemory.call(
      query: "testing",
      memory_type: %w[note session],
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 2, response_data["count"]
    types = response_data["results"].map { |r| r["memory_type"] }

    assert_includes types, "note"
    assert_includes types, "session"
    refute_includes types, "todo"
  end

  def test_filters_by_date_range
    db = @db_manager.get_database(nil)
    id1 = db.store(content: "Old memory about testing")
    id2 = db.store(content: "New memory about testing")

    # Backdate the memories
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", id1]
    )
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", id2]
    )

    result = Recollect::Tools::SearchMemory.call(
      query: "testing",
      created_after: "2025-01-15",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert_equal 1, response_data["count"]
    assert_equal id2, response_data["results"].first["id"]
  end
end
