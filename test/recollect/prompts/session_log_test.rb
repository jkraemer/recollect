# frozen_string_literal: true

require "test_helper"

class SessionLogPromptTest < Recollect::TestCase
  def test_prompt_metadata
    assert_equal "session_log", Recollect::Prompts::SessionLog.name_value
    assert_match(/session summary/, Recollect::Prompts::SessionLog.description_value)
  end

  def test_prompt_arguments
    args = Recollect::Prompts::SessionLog.arguments_value

    assert_equal 1, args.size

    project_arg = args.first

    assert_equal "project", project_arg.name
    refute project_arg.required
  end

  def test_template_without_project
    result = Recollect::Prompts::SessionLog.template({}, server_context: {})

    assert_instance_of MCP::Prompt::Result, result
    assert_equal 1, result.messages.size

    message = result.messages.first

    assert_equal "user", message.role
    assert_match(/Session Log/, message.content.text)
    assert_match(/current project name/, message.content.text)
  end

  def test_template_with_project
    result = Recollect::Prompts::SessionLog.template({project: "myproject"}, server_context: {})

    message = result.messages.first

    assert_match(/"myproject"/, message.content.text)
    refute_match(/current project name/, message.content.text)
  end
end
