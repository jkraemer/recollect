# frozen_string_literal: true

require 'test_helper'

class DeleteMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_deletes_memory_from_global
    db = @db_manager.get_database(nil)
    id = db.store(content: 'To delete')

    result = Recollect::Tools::DeleteMemory.call(
      id: id,
      server_context: { db_manager: @db_manager }
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])
    assert response_data['success']
    assert_equal id, response_data['deleted_id']

    # Verify deleted
    assert_nil db.get(id)
  end

  def test_deletes_memory_from_project
    db = @db_manager.get_database('delete-project')
    id = db.store(content: 'Project memory to delete')

    result = Recollect::Tools::DeleteMemory.call(
      id: id,
      project: 'delete-project',
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    assert response_data['success']
    assert_nil db.get(id)
  end

  def test_returns_failure_for_missing_id
    result = Recollect::Tools::DeleteMemory.call(
      id: 99999,
      server_context: { db_manager: @db_manager }
    )

    response_data = JSON.parse(result.content.first[:text])
    refute response_data['success']
    assert_nil response_data['deleted_id']
  end
end
