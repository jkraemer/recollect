# frozen_string_literal: true

require "test_helper"

class DatabaseManagerTest < Recollect::TestCase
  def setup
    super
    @config = Recollect::Config.new
    @manager = Recollect::DatabaseManager.new(@config)
  end

  def teardown
    @manager.close_all
    super
  end

  # Test get_database returns global database when project is nil
  def test_get_database_returns_global_for_nil
    db = @manager.get_database(nil)

    assert_instance_of Recollect::Database, db
  end

  # Test get_database returns project database
  def test_get_database_returns_project_database
    db = @manager.get_database("my-project")

    assert_instance_of Recollect::Database, db
  end

  # Test get_database caches databases
  def test_get_database_caches_instances
    db1 = @manager.get_database("test-project")
    db2 = @manager.get_database("test-project")

    assert_same db1, db2
  end

  # Test project names are normalized to lowercase
  def test_get_database_normalizes_case
    db1 = @manager.get_database("MyProject")
    db2 = @manager.get_database("myproject")
    db3 = @manager.get_database("MYPROJECT")

    assert_same db1, db2
    assert_same db2, db3
  end

  # Test search results have normalized project names
  def test_search_results_have_normalized_project
    db = @manager.get_database("MixedCase")
    db.store(content: "Test content about search")

    criteria = Recollect::SearchCriteria.new(query: "search", project: "MIXEDCASE")
    results = @manager.search_all(criteria)

    assert_equal 1, results.length
    assert_equal "mixedcase", results.first["project"]
  end

  # Test global and project databases are separate
  def test_global_and_project_databases_are_separate
    global = @manager.get_database(nil)
    project = @manager.get_database("test-project")

    refute_same global, project
  end

  # Test search_all searches specific project only
  def test_search_all_searches_specific_project
    # Store in project
    db = @manager.get_database("search-test")
    db.store(content: "Project memory about Ruby")

    # Store in global
    global = @manager.get_database(nil)
    global.store(content: "Global memory about Ruby")

    # Search specific project
    criteria = Recollect::SearchCriteria.new(query: "Ruby", project: "search-test")
    results = @manager.search_all(criteria)

    assert_equal 1, results.length
    assert_equal "search-test", results.first["project"]
  end

  # Test search_all searches global and all projects when no project specified
  def test_search_all_searches_all_databases
    # Store in global
    global = @manager.get_database(nil)
    global.store(content: "Global memory about testing")

    # Store in project
    db = @manager.get_database("search-all-test")
    db.store(content: "Project memory about testing")

    # Search all
    criteria = Recollect::SearchCriteria.new(query: "testing")
    results = @manager.search_all(criteria)

    assert_equal 2, results.length

    projects = results.map { |r| r["project"] }

    assert_includes projects, nil # global
    assert_includes projects, "search_all_test"
  end

  # Test search_all respects limit
  def test_search_all_respects_limit
    db = @manager.get_database("limit-test")
    5.times { |i| db.store(content: "Memory #{i} about patterns") }

    criteria = Recollect::SearchCriteria.new(query: "patterns", project: "limit-test", limit: 2)
    results = @manager.search_all(criteria)

    assert_equal 2, results.length
  end

  # Test search_all filters by type
  def test_search_all_filters_by_type
    db = @manager.get_database("type-test")
    db.store(content: "A note about bugs", memory_type: "note")
    db.store(content: "A decision about bugs", memory_type: "decision")

    criteria = Recollect::SearchCriteria.new(query: "bugs", project: "type-test", memory_type: "note")
    results = @manager.search_all(criteria)

    assert_equal 1, results.length
    assert_equal "note", results.first["memory_type"]
  end

  # Test search_all filters by date range
  def test_search_all_filters_by_date
    db = @manager.get_database("date-test")
    id1 = db.store(content: "Old memory about testing")
    id2 = db.store(content: "New memory about testing")

    # Backdate the memories
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", id1]
    )
    db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", id2]
    )

    criteria = Recollect::SearchCriteria.new(query: "testing", project: "date-test", created_after: "2025-01-15")
    results = @manager.search_all(criteria)

    assert_equal 1, results.length
    assert_equal id2, results.first["id"]
  end

  # Test list_projects returns empty initially
  def test_list_projects_returns_empty_initially
    projects = @manager.list_projects
    # May have projects from other tests, so just check it's an array
    assert_kind_of Array, projects
  end

  # Test list_projects finds created project databases
  def test_list_projects_finds_projects
    # Create a project database
    @manager.get_database("list-test-project")

    # Force file creation by storing something
    db = @manager.get_database("list-test-project")
    db.store(content: "Test")

    projects = @manager.list_projects

    assert_includes projects, "list_test_project"
  end

  # Test project names are sanitized consistently
  def test_project_name_sanitization
    # All these should map to the same database
    db1 = @manager.get_database("my-project")
    db2 = @manager.get_database("my_project")
    db3 = @manager.get_database("My Project")
    db4 = @manager.get_database("MY-PROJECT")

    assert_same db1, db2, "Hyphen and underscore should map to same db"
    assert_same db1, db3, "Space should be sanitized to underscore"
    assert_same db1, db4, "Should be case-insensitive"
  end

  # Test list_projects returns sorted list
  def test_list_projects_returns_sorted
    @manager.get_database("zzz_project").store(content: "Test")
    @manager.get_database("aaa_project").store(content: "Test")

    projects = @manager.list_projects
    aaa_index = projects.index("aaa_project")
    zzz_index = projects.index("zzz_project")

    assert_operator aaa_index, :<, zzz_index, "Projects should be sorted alphabetically"
  end

  # Test close_all closes all databases
  def test_close_all_closes_databases
    @manager.get_database(nil)
    @manager.get_database("close-test")

    @manager.close_all

    # After close_all, getting database should create new instance
    # (can't easily test if closed, but this exercises the code path)
    assert_kind_of Recollect::Database, @manager.get_database(nil)
  end

  # Test thread safety - concurrent access to get_database
  def test_thread_safety_get_database
    threads = 10.times.map do |i|
      Thread.new do
        5.times { @manager.get_database("thread-test-#{i}") }
      end
    end

    threads.each(&:join)
    # If we got here without errors, thread safety is working
  end

  # Test tag_stats for specific project
  def test_tag_stats_for_specific_project
    db = @manager.get_database("tag-stats-project")
    db.store(content: "Memory 1", tags: %w[ruby testing])
    db.store(content: "Memory 2", tags: %w[ruby performance])
    db.store(content: "Memory 3", tags: ["testing"])

    stats = @manager.tag_stats(project: "tag-stats-project")

    assert_equal 2, stats["ruby"]
    assert_equal 2, stats["testing"]
    assert_equal 1, stats["performance"]
  end

  # Test tag_stats aggregates across all databases
  def test_tag_stats_aggregates_across_all_databases
    # Store in global
    global = @manager.get_database(nil)
    global.store(content: "Global memory", tags: %w[shared global])

    # Store in project
    db = @manager.get_database("tag-stats-aggregate")
    db.store(content: "Project memory", tags: %w[shared project])

    stats = @manager.tag_stats

    assert_equal 2, stats["shared"]
    assert_equal 1, stats["global"]
    assert_equal 1, stats["project"]
  end

  # Test tag_stats with memory_type filter
  def test_tag_stats_with_memory_type_filter
    db = @manager.get_database("tag-stats-type")
    db.store(content: "A note", memory_type: "note", tags: ["note-tag"])
    db.store(content: "A todo", memory_type: "todo", tags: ["todo-tag"])

    stats = @manager.tag_stats(project: "tag-stats-type", memory_type: "note")

    assert_equal 1, stats["note-tag"]
    assert_nil stats["todo-tag"]
  end

  # ========== Hybrid Search Tests ==========

  # Test hybrid_search falls back to FTS5 when vectors unavailable
  def test_hybrid_search_falls_back_to_fts_when_vectors_unavailable
    # Default config has vectors disabled
    db = @manager.get_database("hybrid-fallback")
    db.store(content: "Ruby programming patterns")
    db.store(content: "Python programming patterns")

    criteria = Recollect::SearchCriteria.new(query: "Ruby", project: "hybrid-fallback")
    results = @manager.hybrid_search(criteria)

    assert_equal 1, results.length
    assert_match(/Ruby/, results.first["content"])
    # Should NOT have combined_score since vectors weren't used
    refute results.first.key?("combined_score")
  end

  # Test hybrid_search works with array query (AND semantics)
  def test_hybrid_search_with_array_query
    db = @manager.get_database("hybrid-array")
    db.store(content: "Ruby programming patterns for web apps")
    db.store(content: "Python programming patterns")
    db.store(content: "Ruby web framework comparison")

    criteria = Recollect::SearchCriteria.new(query: %w[Ruby programming], project: "hybrid-array")
    results = @manager.hybrid_search(criteria)

    assert_equal 1, results.length
    assert_match(/Ruby programming/, results.first["content"])
  end

  # ========== Search By Tags Tests ==========

  # Test search_by_tags for specific project
  def test_search_by_tags_for_project
    db = @manager.get_database("tags-project")
    db.store(content: "Memory with ruby tag", tags: ["ruby"])
    db.store(content: "Memory with python tag", tags: ["python"])
    db.store(content: "Memory with both", tags: %w[ruby python])

    criteria = Recollect::SearchCriteria.new(query: ["ruby"], project: "tags-project")
    results = @manager.search_by_tags(criteria)

    assert_equal 2, results.length
    results.each do |result|
      assert_includes result["tags"], "ruby"
      assert_equal "tags-project", result["project"]
    end
  end

  # Test search_by_tags across all projects
  def test_search_by_tags_across_all_projects
    # Store in global
    global = @manager.get_database(nil)
    global.store(content: "Global with shared tag", tags: ["shared"])

    # Store in project
    db = @manager.get_database("tags-all-projects")
    db.store(content: "Project with shared tag", tags: ["shared"])

    criteria = Recollect::SearchCriteria.new(query: ["shared"])
    results = @manager.search_by_tags(criteria)

    assert_equal 2, results.length

    projects = results.map { |r| r["project"] }

    assert_includes projects, nil # global
    assert_includes projects, "tags_all_projects"
  end

  # Test search_by_tags with memory_type filter
  def test_search_by_tags_with_memory_type
    db = @manager.get_database("tags-type")
    db.store(content: "A note with tag", memory_type: "note", tags: ["important"])
    db.store(content: "A decision with tag", memory_type: "decision", tags: ["important"])

    criteria = Recollect::SearchCriteria.new(query: ["important"], project: "tags-type", memory_type: "note")
    results = @manager.search_by_tags(criteria)

    assert_equal 1, results.length
    assert_equal "note", results.first["memory_type"]
  end

  # Test search_by_tags respects limit
  def test_search_by_tags_respects_limit
    db = @manager.get_database("tags-limit")
    5.times { |i| db.store(content: "Memory #{i}", tags: ["common"]) }

    criteria = Recollect::SearchCriteria.new(query: ["common"], project: "tags-limit", limit: 2)
    results = @manager.search_by_tags(criteria)

    assert_equal 2, results.length
  end

  # ========== Store With Embedding Tests ==========

  # Test store_with_embedding returns id
  def test_store_with_embedding_returns_id
    id = @manager.store_with_embedding(
      project: "store-test",
      content: "Test content",
      memory_type: "note",
      tags: %w[test],
      metadata: {key: "value"}
    )

    assert_kind_of Integer, id
    assert_operator id, :>, 0
  end

  # Test store_with_embedding stores content correctly
  def test_store_with_embedding_stores_content
    id = @manager.store_with_embedding(
      project: "store-verify",
      content: "Verify this content",
      memory_type: "decision",
      tags: %w[verify test],
      metadata: {reason: "testing"}
    )

    db = @manager.get_database("store-verify")
    memory = db.get(id)

    assert_equal "Verify this content", memory["content"]
    assert_equal "decision", memory["memory_type"]
    assert_equal %w[verify test], memory["tags"]
  end

  # ========== Enqueue Embedding Tests ==========

  # Test enqueue_embedding does not raise when worker is nil
  def test_enqueue_embedding_noop_when_vectors_disabled
    # Default config has vectors disabled, so @embedding_worker is nil
    # This should not raise
    @manager.enqueue_embedding(memory_id: 1, content: "test", project: "test")
    # If we got here without error, the safe navigation worked
  end

  # ========== Vectors Ready Tests ==========

  # Test vectors_ready? returns false when no databases have vectors
  def test_vectors_ready_false_when_no_vectors
    # Access a database to populate @databases
    @manager.get_database("vectors-ready-test")

    # vectors_ready? should return false since vectors are disabled
    refute @manager.send(:vectors_ready?)
  end
end
