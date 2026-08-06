# frozen_string_literal: true

require "test_helper"

class ResourcesProjectsTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
  end

  def teardown
    @db_manager.close_all
    super
  end

  def build
    Recollect::Resources::Projects.build(db_manager: @db_manager, memories_service: @memories_service)
  end

  def test_builds_a_markdown_resource_per_database_including_global
    @memories_service.create(content: "proj note", project: "alpha", memory_type: "note", tags: [])

    resources = build

    assert_equal ["recollect://project/alpha", "recollect://project/global"],
      resources.map(&:uri_value).sort
    assert(resources.all? { |r| r.mime_type_value == "text/markdown" })
  end

  def test_reading_a_project_resource_renders_its_memories
    @memories_service.create(content: "session summary", project: "alpha", memory_type: "session", tags: [])
    @memories_service.create(content: "open todo", project: "alpha", memory_type: "todo", tags: [])

    resource = build.find { |r| r.uri_value == "recollect://project/alpha" }
    contents = resource.contents

    assert_kind_of MCP::Resource::TextContents, contents
    assert_equal "recollect://project/alpha", contents.uri
    assert_equal "text/markdown", contents.mime_type
    assert_includes contents.text, "# alpha"
    assert_includes contents.text, "session summary"
    assert_includes contents.text, "open todo"
  end

  def test_reading_the_global_resource_queries_the_global_database
    @memories_service.create(content: "global note", project: nil, memory_type: "note", tags: [])

    resource = build.find { |r| r.uri_value == "recollect://project/global" }

    assert_includes resource.contents.text, "global note"
  end
end
