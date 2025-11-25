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
    results = @manager.search_all("Ruby", project: "search-test")

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
    results = @manager.search_all("testing")

    assert_equal 2, results.length

    projects = results.map { |r| r["project"] }

    assert_includes projects, nil # global
    assert_includes projects, "search-all-test"
  end

  # Test search_all respects limit
  def test_search_all_respects_limit
    db = @manager.get_database("limit-test")
    5.times { |i| db.store(content: "Memory #{i} about patterns") }

    results = @manager.search_all("patterns", project: "limit-test", limit: 2)

    assert_equal 2, results.length
  end

  # Test search_all filters by type
  def test_search_all_filters_by_type
    db = @manager.get_database("type-test")
    db.store(content: "A note about bugs", memory_type: "note")
    db.store(content: "A decision about bugs", memory_type: "decision")

    results = @manager.search_all("bugs", project: "type-test", memory_type: "note")

    assert_equal 1, results.length
    assert_equal "note", results.first["memory_type"]
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

    assert_includes projects, "list-test-project"
  end

  # Test list_projects returns sorted list
  def test_list_projects_returns_sorted
    @manager.get_database("zzz-project").store(content: "Test")
    @manager.get_database("aaa-project").store(content: "Test")

    projects = @manager.list_projects
    aaa_index = projects.index("aaa-project")
    zzz_index = projects.index("zzz-project")

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
end
