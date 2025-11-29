# frozen_string_literal: true

require "test_helper"

class StoreMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_stores_memory_in_global_database
    result = Recollect::Tools::StoreMemory.call(
      content: "Test memory",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    assert_kind_of MCP::Tool::Response, result

    # Parse response
    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]
    assert_predicate response_data["id"], :positive?
    assert_equal "global", response_data["stored_in"]
  end

  def test_stores_memory_in_project_database
    result = Recollect::Tools::StoreMemory.call(
      content: "Project memory",
      project: "test-project",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]
    assert_includes response_data["stored_in"], "test-project"
  end

  def test_stores_with_memory_type
    result = Recollect::Tools::StoreMemory.call(
      content: "A decision",
      memory_type: "decision",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]

    # Verify stored correctly
    db = @db_manager.get_database(nil)
    memory = db.get(response_data["id"])

    assert_equal "decision", memory["memory_type"]
  end

  def test_stores_with_tags
    result = Recollect::Tools::StoreMemory.call(
      content: "Tagged memory",
      tags: %w[ruby testing],
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    db = @db_manager.get_database(nil)
    memory = db.get(response_data["id"])

    assert_equal %w[ruby testing], memory["tags"]
  end

  def test_rejects_old_type_decision
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::StoreMemory.input_schema.validate_arguments(
        content: "A decision",
        memory_type: "decision"
      )
    end
  end

  def test_rejects_old_type_pattern
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::StoreMemory.input_schema.validate_arguments(
        content: "A pattern",
        memory_type: "pattern"
      )
    end
  end

  def test_rejects_old_type_bug
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::StoreMemory.input_schema.validate_arguments(
        content: "A bug",
        memory_type: "bug"
      )
    end
  end

  def test_rejects_old_type_learning
    assert_raises(::MCP::Tool::InputSchema::ValidationError) do
      Recollect::Tools::StoreMemory.input_schema.validate_arguments(
        content: "A learning",
        memory_type: "learning"
      )
    end
  end

  def test_accepts_new_type_note
    result = Recollect::Tools::StoreMemory.call(
      content: "A note",
      memory_type: "note",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]
  end

  def test_accepts_new_type_todo
    result = Recollect::Tools::StoreMemory.call(
      content: "A todo",
      memory_type: "todo",
      server_context: { db_manager: @db_manager, memories_service: @memories_service }
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]

    db = @db_manager.get_database(nil)
    memory = db.get(response_data["id"])

    assert_equal "todo", memory["memory_type"]
  end
end
