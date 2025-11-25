# frozen_string_literal: true

require "test_helper"

class DatabaseTest < Recollect::TestCase
  def setup
    super
    @db_path = Pathname.new(ENV.fetch("RECOLLECT_DATA_DIR", nil)).join("test.db")
    @db = Recollect::Database.new(@db_path)
  end

  def teardown
    @db.close
    super
  end

  # Test store returns positive id
  def test_store_returns_id
    id = @db.store(content: "Test memory")

    assert_predicate id, :positive?
  end

  # Test store with all attributes
  def test_store_with_all_attributes
    id = @db.store(
      content: "Test",
      memory_type: "decision",
      tags: %w[ruby test],
      metadata: { key: "value" }
    )

    memory = @db.get(id)

    assert_equal "decision", memory["memory_type"]
    assert_equal %w[ruby test], memory["tags"]
    assert_equal({ "key" => "value" }, memory["metadata"])
  end

  # Test get returns nil for non-existent id
  def test_get_returns_nil_for_missing
    assert_nil @db.get(99_999)
  end

  # Test search finds by content (FTS5)
  def test_search_finds_by_content
    @db.store(content: "Ruby async patterns", memory_type: "pattern")
    @db.store(content: "Python async/await", memory_type: "note")
    @db.store(content: "JavaScript promises", memory_type: "note")

    results = @db.search("async")

    assert_equal 2, results.length
  end

  # Test search filters by type
  def test_search_filters_by_type
    @db.store(content: "Ruby async patterns", memory_type: "pattern")
    @db.store(content: "Python async/await", memory_type: "note")

    results = @db.search("async", memory_type: "pattern")

    assert_equal 1, results.length
    assert_includes results.first["content"], "Ruby"
  end

  # Test search with limit
  def test_search_respects_limit
    5.times { |i| @db.store(content: "Test memory #{i}") }

    results = @db.search("Test", limit: 2)

    assert_equal 2, results.length
  end

  # Test list returns in descending order by created_at
  def test_list_returns_in_descending_order
    3.times { |i| @db.store(content: "Memory #{i}") }

    results = @db.list(limit: 2)

    assert_equal 2, results.length
    assert_equal "Memory 2", results.first["content"]
  end

  # Test list filters by type
  def test_list_filters_by_type
    @db.store(content: "Note 1", memory_type: "note")
    @db.store(content: "Decision 1", memory_type: "decision")
    @db.store(content: "Note 2", memory_type: "note")

    results = @db.list(memory_type: "note")

    assert_equal 2, results.length
    results.each { |r| assert_equal "note", r["memory_type"] }
  end

  # Test list with offset (pagination)
  def test_list_with_offset
    3.times { |i| @db.store(content: "Memory #{i}") }

    results = @db.list(limit: 2, offset: 1)

    assert_equal 2, results.length
    assert_equal "Memory 1", results.first["content"]
  end

  # Test update changes content
  def test_update_changes_content
    id = @db.store(content: "Original")
    @db.update(id, content: "Updated")

    memory = @db.get(id)

    assert_equal "Updated", memory["content"]
  end

  # Test update changes tags
  def test_update_changes_tags
    id = @db.store(content: "Test", tags: ["old"])
    @db.update(id, tags: %w[new tags])

    memory = @db.get(id)

    assert_equal %w[new tags], memory["tags"]
  end

  # Test update returns false for missing id
  def test_update_returns_false_for_missing
    refute @db.update(99_999, content: "Test")
  end

  # Test delete removes memory
  def test_delete_removes_memory
    id = @db.store(content: "To delete")

    assert @db.delete(id)
    assert_nil @db.get(id)
  end

  # Test delete returns false for missing id
  def test_delete_returns_false_for_missing
    refute @db.delete(99_999)
  end

  # Test count returns total
  def test_count_returns_total
    3.times { |i| @db.store(content: "Memory #{i}") }

    assert_equal 3, @db.count
  end

  # Test count filters by type
  def test_count_filters_by_type
    @db.store(content: "Note", memory_type: "note")
    @db.store(content: "Decision", memory_type: "decision")
    @db.store(content: "Note 2", memory_type: "note")

    assert_equal 2, @db.count(memory_type: "note")
    assert_equal 1, @db.count(memory_type: "decision")
  end

  # Test default memory_type is 'note'
  def test_default_memory_type
    id = @db.store(content: "Test")
    memory = @db.get(id)

    assert_equal "note", memory["memory_type"]
  end

  # Test timestamps are set
  def test_timestamps_are_set
    id = @db.store(content: "Test")
    memory = @db.get(id)

    assert memory["created_at"]
    assert memory["updated_at"]
  end

  # Test source is stored
  def test_source_is_stored
    id = @db.store(content: "Test", source: "mcp")
    memory = @db.get(id)

    assert_equal "mcp", memory["source"]
  end

  # Test store rejects empty content
  def test_store_rejects_empty_content
    assert_raises(ArgumentError) { @db.store(content: "") }
  end

  # Test store rejects whitespace-only content
  def test_store_rejects_whitespace_only_content
    assert_raises(ArgumentError) { @db.store(content: "   ") }
  end

  # Test store rejects nil content
  def test_store_rejects_nil_content
    assert_raises(ArgumentError) { @db.store(content: nil) }
  end

  # Test search_by_tags returns memories matching all tags
  def test_search_by_tags_returns_memories_matching_all_tags
    @db.store(content: "Ruby async patterns", tags: %w[ruby async programming])
    @db.store(content: "Ruby basics", tags: %w[ruby beginner])
    @db.store(content: "Python async", tags: %w[python async programming])

    results = @db.search_by_tags(%w[ruby async])

    assert_equal 1, results.length
    assert_includes results.first["content"], "Ruby async patterns"
  end

  # Test search_by_tags with type filter
  def test_search_by_tags_with_type_filter
    @db.store(content: "Ruby pattern", tags: %w[ruby design], memory_type: "pattern")
    @db.store(content: "Ruby note", tags: %w[ruby design], memory_type: "note")
    @db.store(content: "Ruby decision", tags: %w[ruby design], memory_type: "decision")

    results = @db.search_by_tags(%w[ruby design], memory_type: "pattern")

    assert_equal 1, results.length
    assert_equal "pattern", results.first["memory_type"]
  end

  # Test search_by_tags returns empty when no matches
  def test_search_by_tags_returns_empty_when_no_matches
    @db.store(content: "Ruby note", tags: %w[ruby backend])
    @db.store(content: "Python note", tags: %w[python frontend])

    results = @db.search_by_tags(%w[java database])

    assert_empty results
  end

  # Test search_by_tags is case insensitive
  def test_search_by_tags_is_case_insensitive
    @db.store(content: "Ruby patterns", tags: %w[Ruby Programming])
    @db.store(content: "Python patterns", tags: %w[python coding])

    results = @db.search_by_tags(%w[RUBY programming])

    assert_equal 1, results.length
    assert_includes results.first["content"], "Ruby patterns"
  end

  # Test store normalizes tags to lowercase
  def test_store_normalizes_tags_to_lowercase
    id = @db.store(content: "Test", tags: %w[Ruby PYTHON JavaScript])

    memory = @db.get(id)

    assert_equal %w[ruby python javascript], memory["tags"]
  end

  # Test get_tag_stats counts frequency
  def test_get_tag_stats_counts_frequency
    @db.store(content: "Memory 1", tags: %w[ruby programming])
    @db.store(content: "Memory 2", tags: %w[ruby backend])
    @db.store(content: "Memory 3", tags: %w[python programming])
    @db.store(content: "Memory 4", tags: %w[ruby backend programming])

    stats = @db.get_tag_stats

    assert_equal 3, stats["ruby"]
    assert_equal 3, stats["programming"]
    assert_equal 2, stats["backend"]
    assert_equal 1, stats["python"]
  end

  # Test get_tag_stats with type filter
  def test_get_tag_stats_with_type_filter
    @db.store(content: "Note 1", tags: %w[ruby testing], memory_type: "note")
    @db.store(content: "Note 2", tags: %w[ruby debugging], memory_type: "note")
    @db.store(content: "Decision 1", tags: %w[ruby architecture], memory_type: "decision")

    stats = @db.get_tag_stats(memory_type: "note")

    assert_equal 2, stats["ruby"]
    assert_equal 1, stats["testing"]
    assert_equal 1, stats["debugging"]
    assert_nil stats["architecture"]
  end
end
