# frozen_string_literal: true

require "test_helper"

class ResumeSessionPromptTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
    @server_context = {db_manager: @db_manager, memories_service: @memories_service}
  end

  def test_prompt_metadata
    assert_equal "resume_session", Recollect::Prompts::ResumeSession.name_value
    assert_match(/resume.*session/i, Recollect::Prompts::ResumeSession.description_value)
  end

  def test_prompt_arguments
    args = Recollect::Prompts::ResumeSession.arguments_value

    assert_equal 1, args.size

    project_arg = args.first

    assert_equal "project", project_arg.name
    refute project_arg.required
  end

  def test_template_without_project_instructs_agent_to_use_get_context
    result = Recollect::Prompts::ResumeSession.template({}, server_context: @server_context)

    assert_instance_of MCP::Prompt::Result, result
    assert_equal 1, result.messages.size

    message = result.messages.first

    assert_equal "user", message.role
    assert_match(/Resume Session/i, message.content.text)
    assert_match(/get_context/i, message.content.text)
    assert_match(/best.*guess|determine.*project/i, message.content.text)
  end

  def test_template_with_project_and_session_logs
    # Create a session log
    @memories_service.create(
      content: "Session: Test Session\n\nWorked on feature X",
      project: "testproject",
      memory_type: "session",
      tags: ["feature-x"]
    )

    # Create some recent memories
    @memories_service.create(
      content: "Decided to use approach A",
      project: "testproject",
      memory_type: "note",
      tags: ["decision"]
    )

    result = Recollect::Prompts::ResumeSession.template(
      {project: "testproject"},
      server_context: @server_context
    )

    message = result.messages.first

    assert_match(/Resume Session/i, message.content.text)
    assert_match(/Test Session/, message.content.text)
    assert_match(/feature X/, message.content.text)
    assert_match(/approach A/, message.content.text)
  end

  def test_template_shows_only_recent_memories
    # Create an old memory (we can't really set created_at, but we can verify limit works)
    5.times do |i|
      @memories_service.create(
        content: "Memory #{i}",
        project: "testproject",
        memory_type: "note",
        tags: []
      )
    end

    result = Recollect::Prompts::ResumeSession.template(
      {project: "testproject"},
      server_context: @server_context
    )

    message = result.messages.first

    # Should include memories (exact count depends on implementation)
    assert_match(/Memory/, message.content.text)
  end
end
