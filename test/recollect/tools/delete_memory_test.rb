# frozen_string_literal: true

require "test_helper"

class DeleteMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_deletes_memory_from_global
    db = @db_manager.get_database(nil)
    id = db.store(content: "To delete")[:id]
    result = Recollect::Tools::DeleteMemory.call(
      id: id,
      project: nil,
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]
    assert_equal id, response_data["id"]

    # Verify deleted
    assert_nil db.get(id)
  end

  def test_deletes_memory_from_project
    db = @db_manager.get_database("delete-project")
    id = db.store(content: "Project memory to delete")[:id]
    result = Recollect::Tools::DeleteMemory.call(
      id: id,
      project: "delete-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data["success"]
    assert_nil db.get(id)
  end

  def test_returns_failure_for_missing_id
    result = Recollect::Tools::DeleteMemory.call(
      id: 99_999,
      project: nil,
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    refute response_data["success"]
    assert_nil response_data["id"]
  end

  def test_declares_an_output_schema
    schema = Recollect::Tools::DeleteMemory.output_schema.to_h

    assert_equal %w[success id], schema[:required]
  end

  # ids collide across databases, so deleting without naming the project would
  # silently target the global database. The schema must force the choice.
  def test_input_schema_requires_project
    schema = Recollect::Tools::DeleteMemory.input_schema.to_h

    assert_equal %w[id project], schema[:required]
  end

  def test_call_without_project_raises_instead_of_defaulting_to_global
    db = @db_manager.get_database(nil)
    id = db.store(content: "Global memory that must survive")[:id]

    assert_raises(ArgumentError) do
      Recollect::Tools::DeleteMemory.call(
        id: id,
        server_context: {db_manager: @db_manager, memories_service: @memories_service}
      )
    end

    refute_nil db.get(id)
  end
end
