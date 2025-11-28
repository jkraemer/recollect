# frozen_string_literal: true

require "test_helper"

class MemoriesServiceTest < Recollect::TestCase
  def setup
    super
    @config = Recollect::Config.new
    @db_manager = Recollect::DatabaseManager.new(@config)
    @service = Recollect::MemoriesService.new(@db_manager)
  end

  def teardown
    @db_manager.close_all
    super
  end

  # ========== Create ==========

  def test_create_returns_memory_with_id
    result = @service.create(content: "Test memory content")

    assert_kind_of Hash, result
    assert result["id"], "Should return memory with id"
    assert_equal "Test memory content", result["content"]
  end

  def test_create_stores_in_correct_project
    result = @service.create(content: "Project memory", project: "my-project")

    assert_equal "my-project", result["project"]

    # Verify it's actually in that project's database
    db = @db_manager.get_database("my-project")
    memory = db.get(result["id"])

    assert_equal "Project memory", memory["content"]
  end

  def test_create_normalizes_project_to_lowercase
    result = @service.create(content: "Test", project: "MyProject")

    assert_equal "myproject", result["project"]
  end

  def test_create_with_memory_type_and_tags
    result = @service.create(
      content: "Decision content",
      memory_type: "decision",
      tags: %w[architecture api]
    )

    assert_equal "decision", result["memory_type"]
    assert_equal %w[architecture api], result["tags"]
  end

  # ========== Get ==========

  def test_get_returns_memory_by_id
    created = @service.create(content: "Find me")

    result = @service.get(created["id"])

    assert_equal created["id"], result["id"]
    assert_equal "Find me", result["content"]
  end

  def test_get_returns_nil_for_nonexistent_id
    result = @service.get(99_999)

    assert_nil result
  end

  def test_get_from_specific_project
    created = @service.create(content: "Project specific", project: "get-test")

    result = @service.get(created["id"], project: "get-test")

    assert_equal created["id"], result["id"]
    assert_equal "get-test", result["project"]
  end

  def test_get_normalizes_project_to_lowercase
    created = @service.create(content: "Test", project: "myproject")

    # Should find it even with different casing
    result = @service.get(created["id"], project: "MyProject")

    assert_equal created["id"], result["id"]
  end

  # ========== List ==========

  def test_list_returns_memories
    @service.create(content: "Memory 1")
    @service.create(content: "Memory 2")

    result = @service.list

    assert_equal 2, result.length
  end

  def test_list_respects_limit
    5.times { |i| @service.create(content: "Memory #{i}") }

    result = @service.list(limit: 2)

    assert_equal 2, result.length
  end

  def test_list_respects_offset
    3.times { |i| @service.create(content: "Memory #{i}") }

    result = @service.list(limit: 10, offset: 2)

    assert_equal 1, result.length
  end

  def test_list_filters_by_project
    @service.create(content: "Global memory")
    @service.create(content: "Project memory", project: "list-test")

    result = @service.list(project: "list-test")

    assert_equal 1, result.length
    assert_equal "list-test", result.first["project"]
  end

  def test_list_filters_by_memory_type
    @service.create(content: "A note", memory_type: "note")
    @service.create(content: "A todo", memory_type: "todo")

    result = @service.list(memory_type: "note")

    assert_equal 1, result.length
    assert_equal "note", result.first["memory_type"]
  end

  # ========== Delete ==========

  def test_delete_returns_true_on_success
    created = @service.create(content: "Delete me")

    result = @service.delete(created["id"])

    assert result
  end

  def test_delete_returns_false_for_nonexistent
    result = @service.delete(99_999)

    refute result
  end

  def test_delete_removes_memory
    created = @service.create(content: "Delete me")

    @service.delete(created["id"])

    assert_nil @service.get(created["id"])
  end

  def test_delete_from_specific_project
    created = @service.create(content: "Delete from project", project: "delete-test")

    result = @service.delete(created["id"], project: "delete-test")

    assert result
    assert_nil @service.get(created["id"], project: "delete-test")
  end

  # ========== Search ==========

  def test_search_finds_matching_content
    @service.create(content: "Ruby programming patterns")
    @service.create(content: "Python programming patterns")

    results = @service.search("Ruby")

    assert_equal 1, results.length
    assert_match(/Ruby/, results.first["content"])
  end

  def test_search_filters_by_project
    @service.create(content: "Ruby in global")
    @service.create(content: "Ruby in project", project: "search-test")

    results = @service.search("Ruby", project: "search-test")

    assert_equal 1, results.length
    assert_equal "search-test", results.first["project"]
  end

  def test_search_filters_by_memory_type
    @service.create(content: "Ruby note", memory_type: "note")
    @service.create(content: "Ruby todo", memory_type: "todo")

    results = @service.search("Ruby", memory_type: "note")

    assert_equal 1, results.length
    assert_equal "note", results.first["memory_type"]
  end

  def test_search_respects_limit
    5.times { |i| @service.create(content: "Ruby pattern #{i}") }

    results = @service.search("Ruby", limit: 2)

    assert_equal 2, results.length
  end

  # ========== Search By Tags ==========

  def test_search_by_tags_finds_matching
    @service.create(content: "Memory with ruby", tags: ["ruby"])
    @service.create(content: "Memory with python", tags: ["python"])

    results = @service.search_by_tags(["ruby"])

    assert_equal 1, results.length
    assert_includes results.first["tags"], "ruby"
  end

  def test_search_by_tags_uses_and_semantics
    @service.create(content: "Has both", tags: %w[ruby testing])
    @service.create(content: "Has ruby only", tags: ["ruby"])

    results = @service.search_by_tags(%w[ruby testing])

    assert_equal 1, results.length
    assert_equal "Has both", results.first["content"]
  end

  def test_search_by_tags_filters_by_project
    @service.create(content: "Global tagged", tags: ["shared"])
    @service.create(content: "Project tagged", tags: ["shared"], project: "tags-test")

    results = @service.search_by_tags(["shared"], project: "tags-test")

    assert_equal 1, results.length
    assert_equal "tags-test", results.first["project"]
  end

  # ========== List Projects ==========

  def test_list_projects_returns_array
    result = @service.list_projects

    assert_kind_of Array, result
  end

  def test_list_projects_finds_created_projects
    @service.create(content: "Test", project: "project-a")
    @service.create(content: "Test", project: "project-b")

    result = @service.list_projects

    assert_includes result, "project-a"
    assert_includes result, "project-b"
  end

  # ========== Tag Stats ==========

  def test_tag_stats_returns_hash
    @service.create(content: "Tagged", tags: ["ruby"])

    result = @service.tag_stats

    assert_kind_of Hash, result
    assert_equal 1, result["ruby"]
  end

  def test_tag_stats_counts_correctly
    @service.create(content: "Memory 1", tags: %w[ruby testing])
    @service.create(content: "Memory 2", tags: %w[ruby performance])

    result = @service.tag_stats

    assert_equal 2, result["ruby"]
    assert_equal 1, result["testing"]
    assert_equal 1, result["performance"]
  end

  def test_tag_stats_filters_by_project
    @service.create(content: "Global", tags: ["shared"])
    @service.create(content: "Project", tags: ["shared"], project: "stats-test")

    result = @service.tag_stats(project: "stats-test")

    assert_equal 1, result["shared"]
  end
end
