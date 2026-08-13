# frozen_string_literal: true

require "test_helper"

class SchemasTest < Minitest::Test
  SAMPLE_MEMORY = {
    "id" => 1,
    "content" => "We decided to use Puma",
    "memory_type" => "decision",
    "tags" => ["architecture"],
    "project" => "recollect",
    "created_at" => "2026-08-06T10:00:00.000Z",
    "updated_at" => "2026-08-06T10:00:00.000Z",
    "rank" => -0.0001
  }.freeze

  def schema_around_memory
    MCP::Tool::OutputSchema.new(
      "$defs": {memory: Recollect::Schemas::MEMORY},
      properties: {memory: {"$ref": "#/$defs/memory"}},
      required: ["memory"]
    )
  end

  def test_memory_fragment_accepts_a_full_memory_record
    schema_around_memory.validate_result({"memory" => SAMPLE_MEMORY})
  end

  def test_memory_fragment_accepts_global_memory_without_rank
    memory = SAMPLE_MEMORY.except("rank").merge("project" => nil)

    schema_around_memory.validate_result({"memory" => memory})
  end

  def test_memory_fragment_does_not_restrict_memory_type_to_an_enum
    memory = SAMPLE_MEMORY.merge("memory_type" => "learning")

    schema_around_memory.validate_result({"memory" => memory})
  end

  def test_memory_fragment_rejects_a_record_missing_content
    assert_raises(MCP::Tool::OutputSchema::ValidationError) do
      schema_around_memory.validate_result({"memory" => SAMPLE_MEMORY.except("content")})
    end
  end

  # project is part of a memory's identity: ids collide across databases, so a
  # result without its project cannot be safely passed back to delete_memory.
  def test_memory_fragment_rejects_a_record_missing_project
    assert_raises(MCP::Tool::OutputSchema::ValidationError) do
      schema_around_memory.validate_result({"memory" => SAMPLE_MEMORY.except("project")})
    end
  end

  def test_memory_fragment_is_frozen
    assert_predicate Recollect::Schemas::MEMORY, :frozen?
  end
end
