# frozen_string_literal: true

require "test_helper"

class MemoryMarkdownTest < Minitest::Test
  def sample_memory(overrides = {})
    {
      "id" => 7,
      "content" => "Puma stays single-process",
      "memory_type" => "decision",
      "tags" => %w[architecture puma],
      "project" => "recollect",
      "created_at" => "2026-08-06T10:00:00.000Z",
      "updated_at" => "2026-08-06T10:00:00.000Z"
    }.merge(overrides)
  end

  def test_memory_renders_content_and_metadata
    text = Recollect::MemoryMarkdown.memory(sample_memory)

    assert_includes text, "# Memory #7 (decision)"
    assert_includes text, "Puma stays single-process"
    assert_includes text, "architecture, puma"
    assert_includes text, "recollect"
  end

  def test_memory_labels_global_memories
    text = Recollect::MemoryMarkdown.memory(sample_memory("project" => nil, "tags" => []))

    assert_includes text, "global"
    refute_includes text, "Tags:"
  end

  def test_project_context_renders_session_and_recent_memories
    text = Recollect::MemoryMarkdown.project_context(
      name: "recollect",
      last_session: sample_memory("memory_type" => "session", "content" => "Shipped the gem"),
      notes_todos: [sample_memory("id" => 8, "content" => "Widen the mcp constraint", "memory_type" => "todo")]
    )

    assert_includes text, "# recollect"
    assert_includes text, "## Last session"
    assert_includes text, "Shipped the gem"
    assert_includes text, "## Recent notes and todos"
    assert_includes text, "**todo** (#8) Widen the mcp constraint"
  end

  def test_project_context_without_session_or_memories
    text = Recollect::MemoryMarkdown.project_context(name: "empty", last_session: nil, notes_todos: [])

    assert_includes text, "# empty"
    assert_includes text, "No session log yet."
    assert_includes text, "No notes or todos yet."
  end
end
