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

  def test_returns_project_context_structure
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    assert_kind_of MCP::Tool::Response, result

    response_data = JSON.parse(result.content.first[:text])

    assert_equal "context-project", response_data["project"]
    assert response_data.key?("last_session"), "Should have last_session key"
    assert response_data.key?("recent_notes_todos"), "Should have recent_notes_todos key"
  end

  def test_returns_notes_in_recent_notes_todos
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])
    notes_todos = response_data["recent_notes_todos"]

    # Setup created 2 notes (decision and pattern types won't be included - only note/todo)
    assert_equal 2, notes_todos.length
    assert notes_todos.all? { |m| %w[note todo].include?(m["memory_type"]) }
  end

  def test_returns_null_last_session_when_none_exists
    result = Recollect::Tools::GetContext.call(
      project: "context-project",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    # Setup has no sessions, so last_session should be nil
    assert_nil response_data["last_session"]
  end

  def test_project_parameter_is_optional
    # Create memories in multiple projects
    @db_manager.get_database("project-a").store(content: "Memory A", memory_type: "note")
    @db_manager.get_database("project-b").store(content: "Memory B", memory_type: "note")

    result = Recollect::Tools::GetContext.call(
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    assert_kind_of MCP::Tool::Response, result
    response_data = JSON.parse(result.content.first[:text])

    # Should return cross-project results when no project specified
    assert_nil response_data["project"]
  end

  def test_returns_last_session_for_project
    db = @db_manager.get_database("session-test")
    db.store(content: "Old session", memory_type: "session")
    db.store(content: "Latest session", memory_type: "session")

    result = Recollect::Tools::GetContext.call(
      project: "session-test",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data.key?("last_session"), "Should have last_session key"
    assert_equal "Latest session", response_data["last_session"]["content"]
  end

  def test_returns_recent_notes_and_todos
    db = @db_manager.get_database("notes-test")
    # Create more than 10 to verify limit
    12.times { |i| db.store(content: "Note #{i}", memory_type: "note") }
    db.store(content: "A todo item", memory_type: "todo")
    db.store(content: "A session", memory_type: "session") # Should not be included

    result = Recollect::Tools::GetContext.call(
      project: "notes-test",
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    assert response_data.key?("recent_notes_todos"), "Should have recent_notes_todos key"
    notes_todos = response_data["recent_notes_todos"]

    assert_equal 10, notes_todos.length, "Should limit to 10 notes/todos"
    # Verify no sessions in notes_todos
    assert notes_todos.none? { |m| m["memory_type"] == "session" }
  end

  def test_cross_project_returns_recent_activity
    # Create memories in multiple projects with different timestamps
    @db_manager.get_database("proj-a").store(content: "Session A", memory_type: "session")
    @db_manager.get_database("proj-a").store(content: "Note A", memory_type: "note")
    @db_manager.get_database("proj-b").store(content: "Session B", memory_type: "session")
    @db_manager.get_database("proj-b").store(content: "Note B", memory_type: "note")
    @db_manager.get_database(nil).store(content: "Global note", memory_type: "note") # global

    result = Recollect::Tools::GetContext.call(
      server_context: {db_manager: @db_manager, memories_service: @memories_service}
    )

    response_data = JSON.parse(result.content.first[:text])

    # Should include results from multiple projects
    assert response_data.key?("recent_sessions"), "Should have recent_sessions key"
    assert response_data.key?("recent_notes_todos"), "Should have recent_notes_todos key"

    # Should have sessions from both projects
    sessions = response_data["recent_sessions"]
    projects_with_sessions = sessions.map { |s| s["project"] }.uniq

    assert_operator projects_with_sessions.length, :>=, 2
  end
end
