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

    criteria = Recollect::SearchCriteria.new(query: "Ruby")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_match(/Ruby/, results.first["content"])
  end

  def test_search_filters_by_project
    @service.create(content: "Ruby in global")
    @service.create(content: "Ruby in project", project: "search-test")

    criteria = Recollect::SearchCriteria.new(query: "Ruby", project: "search-test")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_equal "search-test", results.first["project"]
  end

  def test_search_filters_by_memory_type
    @service.create(content: "Ruby note", memory_type: "note")
    @service.create(content: "Ruby todo", memory_type: "todo")

    criteria = Recollect::SearchCriteria.new(query: "Ruby", memory_type: "note")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_equal "note", results.first["memory_type"]
  end

  def test_search_respects_limit
    5.times { |i| @service.create(content: "Ruby pattern #{i}") }

    criteria = Recollect::SearchCriteria.new(query: "Ruby", limit: 2)
    results = @service.search(criteria)

    assert_equal 2, results.length
  end

  # ========== Search with Date Filtering ==========

  def test_search_filters_by_created_after
    old = @service.create(content: "Old Ruby memory")
    new = @service.create(content: "New Ruby memory")

    # Backdate the old memory
    db = @db_manager.get_database(nil)
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", old["id"]]
    )

    criteria = Recollect::SearchCriteria.new(query: "Ruby", created_after: "2025-01-15")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_equal new["id"], results.first["id"]
  end

  def test_search_filters_by_created_before
    old = @service.create(content: "Old Ruby memory")
    new = @service.create(content: "New Ruby memory")

    # Backdate the old memory
    db = @db_manager.get_database(nil)
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", old["id"]]
    )
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", new["id"]]
    )

    criteria = Recollect::SearchCriteria.new(query: "Ruby", created_before: "2025-01-15")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_equal old["id"], results.first["id"]
  end

  def test_search_filters_by_date_range
    m1 = @service.create(content: "Ruby January")
    m2 = @service.create(content: "Ruby February")
    m3 = @service.create(content: "Ruby March")

    db = @db_manager.get_database(nil)
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-15T00:00:00Z", m1["id"]]
    )
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-02-15T00:00:00Z", m2["id"]]
    )
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-03-15T00:00:00Z", m3["id"]]
    )

    criteria = Recollect::SearchCriteria.new(query: "Ruby", created_after: "2025-02-01", created_before: "2025-02-28")
    results = @service.search(criteria)

    assert_equal 1, results.length
    assert_equal m2["id"], results.first["id"]
  end

  # ========== Search By Tags ==========

  def test_search_by_tags_finds_matching
    @service.create(content: "Memory with ruby", tags: ["ruby"])
    @service.create(content: "Memory with python", tags: ["python"])

    criteria = Recollect::SearchCriteria.new(query: ["ruby"])
    results = @service.search_by_tags(criteria)

    assert_equal 1, results.length
    assert_includes results.first["tags"], "ruby"
  end

  def test_search_by_tags_uses_and_semantics
    @service.create(content: "Has both", tags: %w[ruby testing])
    @service.create(content: "Has ruby only", tags: ["ruby"])

    criteria = Recollect::SearchCriteria.new(query: %w[ruby testing])
    results = @service.search_by_tags(criteria)

    assert_equal 1, results.length
    assert_equal "Has both", results.first["content"]
  end

  def test_search_by_tags_filters_by_project
    @service.create(content: "Global tagged", tags: ["shared"])
    @service.create(content: "Project tagged", tags: ["shared"], project: "tags-test")

    criteria = Recollect::SearchCriteria.new(query: ["shared"], project: "tags-test")
    results = @service.search_by_tags(criteria)

    assert_equal 1, results.length
    assert_equal "tags-test", results.first["project"]
  end

  def test_search_by_tags_filters_by_date
    old = @service.create(content: "Old tagged", tags: ["ruby"])
    new = @service.create(content: "New tagged", tags: ["ruby"])

    db = @db_manager.get_database(nil)
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", old["id"]]
    )

    criteria = Recollect::SearchCriteria.new(query: ["ruby"], created_after: "2025-01-15")
    results = @service.search_by_tags(criteria)

    assert_equal 1, results.length
    assert_equal new["id"], results.first["id"]
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

    assert_includes result, "project_a"
    assert_includes result, "project_b"
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

  # ========== List All Projects ==========

  def test_list_all_returns_memories_from_all_projects
    @service.create(content: "Global memory")
    @service.create(content: "Project A memory", project: "project-a")
    @service.create(content: "Project B memory", project: "project-b")

    result = @service.list_all

    assert_equal 3, result.length

    projects = result.map { |m| m["project"] }

    assert_includes projects, nil
    assert_includes projects, "project_a"
    assert_includes projects, "project_b"
  end

  def test_list_all_respects_limit
    5.times { |i| @service.create(content: "Memory #{i}") }
    3.times { |i| @service.create(content: "Project memory #{i}", project: "proj") }

    result = @service.list_all(limit: 4)

    assert_equal 4, result.length
  end

  # ========== Embedding Status ==========

  def test_list_does_not_include_has_embedding_when_vectors_disabled
    skip_if_vectors_enabled

    @service.create(content: "Test memory")

    result = @service.list

    refute result.first.key?("has_embedding"), "Should not include has_embedding when vectors disabled"
  end

  def test_list_includes_has_embedding_when_vectors_enabled
    skip_unless_vectors_enabled

    @service.create(content: "Test memory")

    result = @service.list

    assert result.first.key?("has_embedding"), "Should include has_embedding when vectors enabled"
  end

  def test_get_does_not_include_has_embedding_when_vectors_disabled
    skip_if_vectors_enabled

    created = @service.create(content: "Test memory")

    result = @service.get(created["id"])

    refute result.key?("has_embedding"), "Should not include has_embedding when vectors disabled"
  end

  def test_get_includes_has_embedding_when_vectors_enabled
    skip_unless_vectors_enabled

    created = @service.create(content: "Test memory")

    result = @service.get(created["id"])

    assert result.key?("has_embedding"), "Should include has_embedding when vectors enabled"
  end

  private

  def skip_unless_vectors_enabled
    skip "Vectors not enabled" unless @config.vectors_available?
  end

  def skip_if_vectors_enabled
    skip "Vectors are enabled" if @config.vectors_available?
  end
end
