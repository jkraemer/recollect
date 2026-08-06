# frozen_string_literal: true

require "test_helper"

class ResourcesMemoryTest < Recollect::TestCase
  def setup
    super
    @db_manager = Recollect::DatabaseManager.new
    @memories_service = Recollect::MemoriesService.new(@db_manager)
    @template = Recollect::Resources::Memory.build(
      db_manager: @db_manager, memories_service: @memories_service
    )
  end

  def teardown
    @db_manager.close_all
    super
  end

  def test_template_matches_the_memory_uri_shape
    assert_equal "recollect://project/{project}/memory/{id}", @template.uri_template
    assert_equal({project: "alpha", id: "3"},
      @template.match_uri("recollect://project/alpha/memory/3"))
  end

  def test_resolves_a_project_memory_as_markdown
    memory = @memories_service.create(content: "resolved body", project: "alpha", memory_type: "note", tags: [])
    contents = @template.contents(project: "alpha", id: memory["id"].to_s)

    assert_kind_of MCP::Resource::TextContents, contents
    assert_equal "text/markdown", contents.mime_type
    assert_equal "recollect://project/alpha/memory/#{memory["id"]}", contents.uri
    assert_includes contents.text, "resolved body"
  end

  def test_resolves_a_global_memory
    memory = @memories_service.create(content: "global body", project: nil, memory_type: "note", tags: [])
    contents = @template.contents(project: "global", id: memory["id"].to_s)

    assert_includes contents.text, "global body"
  end

  def test_unknown_project_raises_resource_not_found_without_creating_a_database
    assert_raises(MCP::Server::ResourceNotFoundError) do
      @template.contents(project: "ghost", id: "1")
    end
    refute_includes @db_manager.list_db_names, "ghost"
  end

  def test_unknown_id_raises_resource_not_found
    @memories_service.create(content: "exists", project: "alpha", memory_type: "note", tags: [])

    assert_raises(MCP::Server::ResourceNotFoundError) do
      @template.contents(project: "alpha", id: "999")
    end
  end

  def test_non_numeric_id_raises_resource_not_found
    @memories_service.create(content: "exists", project: "alpha", memory_type: "note", tags: [])

    assert_raises(MCP::Server::ResourceNotFoundError) do
      @template.contents(project: "alpha", id: "abc")
    end
  end
end
