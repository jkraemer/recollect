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
    id = @db.store(content: "Test memory")[:id]

    assert_predicate id, :positive?
  end

  # Test store with all attributes
  def test_store_with_all_attributes
    id = @db.store(
      content: "Test",
      memory_type: "decision",
      tags: %w[ruby test],
      metadata: {key: "value"}
    )[:id]

    memory = @db.get(id)

    assert_equal "decision", memory["memory_type"]
    assert_equal %w[ruby test], memory["tags"]
    assert_equal({"key" => "value"}, memory["metadata"])
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

  # Test list filters by array of types
  def test_list_filters_by_array_of_types
    @db.store(content: "Note 1", memory_type: "note")
    @db.store(content: "Todo 1", memory_type: "todo")
    @db.store(content: "Session 1", memory_type: "session")
    @db.store(content: "Note 2", memory_type: "note")

    results = @db.list(memory_type: %w[note todo])

    assert_equal 3, results.length
    results.each { |r| assert_includes %w[note todo], r["memory_type"] }
  end

  # Test list with offset (pagination)
  def test_list_with_offset
    3.times { |i| @db.store(content: "Memory #{i}") }

    results = @db.list(limit: 2, offset: 1)

    assert_equal 2, results.length
    assert_equal "Memory 1", results.first["content"]
  end

  # Test delete removes memory
  def test_delete_removes_memory
    id = @db.store(content: "To delete")[:id]

    assert @db.delete(id)
    assert_nil @db.get(id)
  end

  # Test delete returns false for missing id
  def test_delete_returns_false_for_missing
    refute @db.delete(99_999)
  end

  def test_delete_tombstones_row_instead_of_hard_delete
    id = @db.store(content: "doomed", memory_type: "note")[:id]
    @db.delete(id, deleted_by_peer: "local")
    raw = @db.instance_variable_get(:@db)
    row = raw.get_first_row("SELECT id, content, deleted_at, deleted_by_peer FROM memories WHERE id = ?", id)

    refute_nil row, "row should still exist physically (tombstone, not hard delete)"
    refute_nil row["deleted_at"], "deleted_at should be set"
    assert_equal "local", row["deleted_by_peer"]
  end

  def test_delete_returns_true_when_row_existed
    id = @db.store(content: "x", memory_type: "note")[:id]

    assert(@db.delete(id, deleted_by_peer: "local"))
  end

  def test_delete_returns_false_when_row_missing
    refute(@db.delete(99_999, deleted_by_peer: "local"))
  end

  def test_delete_idempotent_on_already_deleted
    id = @db.store(content: "x", memory_type: "note")[:id]
    @db.delete(id, deleted_by_peer: "local")
    # Second call should not raise; should be no-op (returns false because deleted_at is unchanged)
    refute(@db.delete(id, deleted_by_peer: "local"))
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

  def test_count_excludes_tombstoned
    @db.store(content: "alive", memory_type: "note")
    dead = @db.store(content: "dead", memory_type: "note")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", dead]
    )

    assert_equal 1, @db.count
  end

  # Test default memory_type is 'note'
  def test_default_memory_type
    id = @db.store(content: "Test")[:id]
    memory = @db.get(id)

    assert_equal "note", memory["memory_type"]
  end

  # Test timestamps are set
  def test_timestamps_are_set
    id = @db.store(content: "Test")[:id]
    memory = @db.get(id)

    assert memory["created_at"]
    assert memory["updated_at"]
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
    id = @db.store(content: "Test", tags: %w[Ruby PYTHON JavaScript])[:id]
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

  # Test search_by_tags returns empty for nil or empty tags
  def test_search_by_tags_returns_empty_for_nil_tags
    @db.store(content: "Test", tags: %w[foo bar])

    assert_empty @db.search_by_tags(nil)
    assert_empty @db.search_by_tags([])
  end

  # Test get_tag_stats skips nil tags gracefully
  def test_get_tag_stats_handles_nil_tags
    # Create a memory with tags
    @db.store(content: "Has tags", tags: %w[test])

    # Manually insert a row with nil tags to test edge case
    @db.instance_variable_get(:@db).execute(
      "INSERT INTO memories (content, memory_type, tags) VALUES (?, ?, ?)",
      ["No tags", "note", nil]
    )

    stats = @db.get_tag_stats

    # Should still work
    assert_equal 1, stats["test"]
  end

  # Test search_by_tags respects limit
  def test_search_by_tags_respects_limit
    5.times { |i| @db.store(content: "Memory #{i}", tags: ["common"]) }

    results = @db.search_by_tags(["common"], limit: 2)

    assert_equal 2, results.length
  end

  # Test search_by_tags handles special characters in tags
  def test_search_by_tags_with_special_characters
    @db.store(content: "Bug fix memory", tags: ['bug"fix', "testing"])
    @db.store(content: "Normal memory", tags: %w[bugfix testing])

    # Should find the memory with the quoted tag
    results = @db.search_by_tags(['bug"fix'])

    assert_equal 1, results.length
    assert_equal "Bug fix memory", results.first["content"]
  end

  # Test search_by_tags handles backslash in tags
  def test_search_by_tags_with_backslash
    @db.store(content: "Path memory", tags: ['path\\to\\file', "filesystem"])
    @db.store(content: "Other memory", tags: %w[path other])

    results = @db.search_by_tags(['path\\to\\file'])

    assert_equal 1, results.length
    assert_equal "Path memory", results.first["content"]
  end

  # Test search with offset
  def test_search_respects_offset
    5.times { |i| @db.store(content: "Test search #{i}") }

    results = @db.search("Test search", limit: 10, offset: 2)

    # Should skip first 2 results
    assert_operator results.length, :<=, 3
  end

  # Test search with array of terms matches ALL terms (AND semantics)
  def test_search_with_array_matches_all_terms
    @db.store(content: "Ruby async patterns for web apps")
    @db.store(content: "Python async/await syntax")
    @db.store(content: "Ruby web framework comparison")
    @db.store(content: "JavaScript async promises")

    results = @db.search(%w[Ruby async])

    assert_equal 1, results.length
    assert_includes results.first["content"], "Ruby async"
  end

  # Test search with array terms can appear in any order
  def test_search_with_array_terms_any_order
    @db.store(content: "Web development with Ruby on Rails")
    @db.store(content: "Ruby gems for web scraping")

    results = @db.search(%w[web Ruby])

    assert_equal 2, results.length
  end

  # Test search with single-element array works like phrase
  def test_search_with_single_element_array
    @db.store(content: "Ruby async patterns")
    @db.store(content: "Python async")

    results = @db.search(["async"])

    assert_equal 2, results.length
  end

  # Test search with created_after date filter
  def test_search_with_created_after
    id1 = @db.store(content: "Old memory about Ruby")[:id]
    id2 = @db.store(content: "New memory about Ruby")[:id]
    # Backdate the first memory
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", id1]
    )
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", id2]
    )

    results = @db.search("Ruby", created_after: "2025-01-15")

    assert_equal 1, results.length
    assert_equal id2, results.first["id"]
  end

  # Test search with created_before date filter
  def test_search_with_created_before
    id1 = @db.store(content: "Old memory about Ruby")[:id]
    id2 = @db.store(content: "New memory about Ruby")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", id1]
    )
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", id2]
    )

    results = @db.search("Ruby", created_before: "2025-01-15")

    assert_equal 1, results.length
    assert_equal id1, results.first["id"]
  end

  # Test search with both date filters (date range)
  def test_search_with_date_range
    id1 = @db.store(content: "January memory about Ruby")[:id]
    id2 = @db.store(content: "February memory about Ruby")[:id]
    id3 = @db.store(content: "March memory about Ruby")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-15T00:00:00Z", id1]
    )
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-02-15T00:00:00Z", id2]
    )
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-03-15T00:00:00Z", id3]
    )

    results = @db.search("Ruby", created_after: "2025-02-01", created_before: "2025-02-28")

    assert_equal 1, results.length
    assert_equal id2, results.first["id"]
  end

  # Test search with wildcard returns all records in reverse chronological order
  def test_search_wildcard_returns_all_records
    @db.store(content: "First memory")
    @db.store(content: "Second memory")
    @db.store(content: "Third memory")

    results = @db.search("*")

    assert_equal 3, results.length
    # Newest first
    assert_equal "Third memory", results.first["content"]
    assert_equal "First memory", results.last["content"]
  end

  # Test wildcard with memory_type filter
  def test_search_wildcard_with_memory_type_filter
    @db.store(content: "Note 1", memory_type: "note")
    @db.store(content: "Decision 1", memory_type: "decision")
    @db.store(content: "Note 2", memory_type: "note")

    results = @db.search("*", memory_type: "note")

    assert_equal 2, results.length
    results.each { |r| assert_equal "note", r["memory_type"] }
  end

  # Test wildcard with date filters
  def test_search_wildcard_with_date_filters
    id1 = @db.store(content: "Old memory")[:id]
    id2 = @db.store(content: "New memory")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-01T00:00:00Z", id1]
    )
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET created_at = ? WHERE id = ?",
      ["2025-01-20T00:00:00Z", id2]
    )

    results = @db.search("*", created_after: "2025-01-15")

    assert_equal 1, results.length
    assert_equal id2, results.first["id"]
  end

  # Test wildcard respects limit and offset
  def test_search_wildcard_respects_limit_and_offset
    5.times { |i| @db.store(content: "Memory #{i}") }

    results = @db.search("*", limit: 2, offset: 1)

    assert_equal 2, results.length
    # Skip newest (Memory 4), get Memory 3 and Memory 2
    assert_equal "Memory 3", results.first["content"]
    assert_equal "Memory 2", results.last["content"]
  end

  def test_memories_table_has_sync_columns
    cols = @db.instance_variable_get(:@db).execute("PRAGMA table_info(memories)")
    names = cols.map { |c| c["name"] }

    assert_includes names, "global_id"
    assert_includes names, "origin_peer"
    assert_includes names, "deleted_at"
    assert_includes names, "deleted_by_peer"
  end

  def test_store_assigns_global_id_and_origin_peer
    id = @db.store(content: "hello", memory_type: "note", origin_peer: "test-peer")[:id]
    row = @db.instance_variable_get(:@db).get_first_row("SELECT global_id, origin_peer FROM memories WHERE id = ?", id)

    assert_match(/\A[0-9a-f-]{36}\z/, row["global_id"])
    assert_equal "test-peer", row["origin_peer"]
  end

  def test_store_default_origin_peer_is_local
    id = @db.store(content: "hello", memory_type: "note")[:id]
    row = @db.instance_variable_get(:@db).get_first_row("SELECT origin_peer FROM memories WHERE id = ?", id)

    assert_equal "local", row["origin_peer"]
  end

  def test_global_id_is_unique_when_present
    @db.instance_variable_get(:@db).execute(
      "INSERT INTO memories (content, memory_type, global_id) VALUES (?, ?, ?)",
      ["a", "note", "uuid-x"]
    )
    assert_raises(SQLite3::ConstraintException) do
      @db.instance_variable_get(:@db).execute(
        "INSERT INTO memories (content, memory_type, global_id) VALUES (?, ?, ?)",
        ["b", "note", "uuid-x"]
      )
    end
  end

  def test_tombstone_removes_row_from_fts
    id = @db.store(content: "haystack needle marker", memory_type: "note")[:id]
    # Confirm searchable
    refute_empty @db.search("marker")
    # Tombstone
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", id]
    )
    # FTS index entry removed (memories_fts_docsize is the FTS5 shadow table
    # that tracks indexed rows; querying memories_fts directly reflects the
    # underlying content table, which still has the soft-deleted row).
    docsize_count = @db.instance_variable_get(:@db).get_first_value(
      "SELECT COUNT(*) FROM memories_fts_docsize WHERE id = ?", id
    )

    assert_equal 0, docsize_count
  end

  def test_tombstone_does_not_corrupt_fts_when_other_columns_unchanged
    # Soft-delete via UPDATE that touches only deleted_at fields.
    # Both memories_au (now WHEN-guarded) and memories_tombstone_fts will be considered;
    # only memories_tombstone_fts should fire. FTS must stay consistent.
    id1 = @db.store(content: "alpha bravo charlie", memory_type: "note")[:id]
    id2 = @db.store(content: "alpha delta echo", memory_type: "note")[:id]
    raw = @db.instance_variable_get(:@db)
    raw.execute("UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", id1])

    # FTS still answers correctly for the surviving row
    results = @db.search("alpha")
    ids = results.map { |r| r["id"] }

    refute_includes ids, id1
    assert_includes ids, id2
  end

  def test_get_returns_nil_for_tombstoned
    id = @db.store(content: "x", memory_type: "note")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", id]
    )

    assert_nil @db.get(id)
  end

  def test_list_excludes_tombstoned
    alive = @db.store(content: "alive", memory_type: "note")[:id]
    dead = @db.store(content: "dead", memory_type: "note")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", dead]
    )
    ids = @db.list.map { |r| r["id"] }

    assert_includes ids, alive
    refute_includes ids, dead
  end

  def test_search_excludes_tombstoned
    alive = @db.store(content: "needle alive", memory_type: "note")[:id]
    dead = @db.store(content: "needle dead", memory_type: "note")[:id]
    @db.instance_variable_get(:@db).execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", dead]
    )
    ids = @db.search("needle").map { |r| r["id"] }

    assert_includes ids, alive
    refute_includes ids, dead
  end

  def test_backfill_origin_peer_fills_null_rows
    raw = @db.instance_variable_get(:@db)
    # Insert a row that bypasses the normal store path (simulating legacy data)
    raw.execute("INSERT INTO memories (content, memory_type) VALUES ('legacy', 'note')")
    legacy_id = raw.last_insert_row_id

    @db.backfill_origin_peer!("peer-abc")
    row = raw.get_first_row("SELECT global_id, origin_peer FROM memories WHERE id = ?", legacy_id)

    refute_nil row["global_id"]
    assert_equal "peer-abc", row["origin_peer"]
  end

  def test_backfill_does_not_clobber_existing_origin_peer
    id = @db.store(content: "x", memory_type: "note", origin_peer: "other-peer")[:id]
    @db.backfill_origin_peer!("local-peer")
    row = @db.instance_variable_get(:@db).get_first_row("SELECT origin_peer FROM memories WHERE id = ?", id)

    assert_equal "other-peer", row["origin_peer"]
  end

  def test_store_returns_id_and_global_id
    result = @db.store(content: "hi", memory_type: "note")

    assert_kind_of Integer, result[:id]
    assert_match(/\A[0-9a-f-]{36}\z/, result[:global_id])
  end

  def test_resurrection_is_rejected_and_fts_remains_intact
    raw = @db.instance_variable_get(:@db)
    id = @db.store(content: "marker", memory_type: "note")[:id]
    # Tombstone the row
    raw.execute("UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [Time.now.utc.iso8601, "local", id])

    refute_empty raw.execute("SELECT 1 FROM memories WHERE id = ?", id)

    # Attempt to resurrect — must be rejected
    assert_raises(SQLite3::ConstraintException) do
      raw.execute("UPDATE memories SET deleted_at = NULL, deleted_by_peer = NULL WHERE id = ?", [id])
    end

    # The row remains tombstoned and FTS is still consistent (no corruption)
    row = raw.get_first_row("SELECT deleted_at FROM memories WHERE id = ?", id)

    refute_nil row["deleted_at"]
    # FTS index lookup must not error (would error if corrupted)
    assert_equal 0, raw.get_first_value("SELECT COUNT(*) FROM memories_fts_docsize WHERE id = ?", id)
  end

  def test_fetch_for_sync_excludes_chunks
    parent = @db.store(content: "parent", memory_type: "note")
    @db.instance_variable_get(:@db).execute(
      "INSERT INTO memories (content, memory_type, global_id, origin_peer) VALUES (?, ?, ?, ?)",
      ["chunk", "_chunk", SecureRandom.uuid_v7, "local"]
    )
    rows = @db.fetch_for_sync(since: {}, limit: 10)

    assert_equal 1, rows.size
    assert_equal parent[:global_id], rows.first["global_id"]
  end

  def test_fetch_for_sync_respects_since_per_origin
    raw = @db.instance_variable_get(:@db)
    # Three rows: two from peer-a (one old, one new), one from peer-b
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["a1", "note", "g-a1", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["a2", "note", "g-a2", "peer-a", "2026-05-02T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["b1", "note", "g-b1", "peer-b", "2026-05-01T12:00:00.000Z"])

    cursor = {"created_at" => "2026-05-01T00:00:00.000Z", "global_id" => "g-a1"}
    rows = @db.fetch_for_sync(since: {"peer-a" => cursor}, limit: 10)
    global_ids = rows.map { |r| r["global_id"] }

    refute_includes global_ids, "g-a1", "a1 IS the cursor — must be excluded (strictly greater)"
    assert_includes global_ids, "g-a2"
    assert_includes global_ids, "g-b1", "peer-b has no cursor, gets everything"
  end

  def test_fetch_for_sync_breaks_ties_on_global_id_within_same_created_at
    raw = @db.instance_variable_get(:@db)
    # Three peer-a rows at the SAME millisecond, distinct global_ids
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["x", "note", "g-aaa", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["y", "note", "g-bbb", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["z", "note", "g-ccc", "peer-a", "2026-05-01T00:00:00.000Z"])

    cursor = {"created_at" => "2026-05-01T00:00:00.000Z", "global_id" => "g-aaa"}
    rows = @db.fetch_for_sync(since: {"peer-a" => cursor}, limit: 10)
    ids = rows.map { |r| r["global_id"] }

    refute_includes ids, "g-aaa", "g-aaa IS the cursor"
    assert_includes ids, "g-bbb", "g-bbb > g-aaa lex, same ts"
    assert_includes ids, "g-ccc", "g-ccc > g-aaa lex, same ts"
  end

  def test_fetch_for_sync_includes_tombstones
    result = @db.store(content: "doomed", memory_type: "note")
    @db.delete(result[:id], deleted_by_peer: "local")
    rows = @db.fetch_for_sync(since: {}, limit: 10)

    assert_equal 1, rows.size
    refute_nil rows.first["deleted_at"]
  end

  def test_fetch_for_sync_pagination_with_composite_cursor
    6.times { |i| @db.store(content: "m#{i}", memory_type: "note") }
    page1 = @db.fetch_for_sync(since: {}, limit: 3)

    assert_equal 3, page1.size
    last = page1.last
    cursor = {"created_at" => last["created_at"], "global_id" => last["global_id"]}
    page2 = @db.fetch_for_sync(since: {"local" => cursor}, limit: 3)

    assert_equal 3, page2.size, "all 3 remaining records should come back even when same-ms"
    assert_empty page1.map { |r| r["global_id"] } & page2.map { |r| r["global_id"] }, "no overlap"
  end

  def test_max_origin_cursor_returns_composite_for_origin
    raw = @db.instance_variable_get(:@db)
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["a", "note", "g1", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["b", "note", "g2", "peer-a", "2026-05-03T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["c", "note", "g3", "peer-b", "2026-05-02T00:00:00.000Z"])

    cursor_a = @db.max_origin_cursor("peer-a")

    assert_equal "2026-05-03T00:00:00.000Z", cursor_a["created_at"]
    assert_equal "g2", cursor_a["global_id"]

    cursor_b = @db.max_origin_cursor("peer-b")

    assert_equal "2026-05-02T00:00:00.000Z", cursor_b["created_at"]
    assert_equal "g3", cursor_b["global_id"]
  end

  def test_max_origin_cursor_returns_nil_when_no_rows
    assert_nil @db.max_origin_cursor("peer-missing")
  end

  def test_max_origin_cursor_breaks_ties_on_global_id_at_same_created_at
    raw = @db.instance_variable_get(:@db)
    # Three peer-a rows at the SAME ms; max cursor must be the largest global_id.
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["x", "note", "g-aaa", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["y", "note", "g-ccc", "peer-a", "2026-05-01T00:00:00.000Z"])
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["z", "note", "g-bbb", "peer-a", "2026-05-01T00:00:00.000Z"])

    cursor = @db.max_origin_cursor("peer-a")

    assert_equal "2026-05-01T00:00:00.000Z", cursor["created_at"]
    assert_equal "g-ccc", cursor["global_id"], "tiebreak on max global_id when created_at ties"
  end

  def test_max_origin_cursor_includes_tombstoned_rows
    # Tombstones still count for cursor purposes — peers need to know about them.
    result = @db.store(content: "doomed", memory_type: "note", origin_peer: "peer-x")
    @db.delete(result[:id], deleted_by_peer: "peer-x")
    cursor = @db.max_origin_cursor("peer-x")

    refute_nil cursor
    assert_equal result[:global_id], cursor["global_id"]
  end

  def test_max_origin_cursor_excludes_chunks
    raw = @db.instance_variable_get(:@db)
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["parent", "note", "g-p", "peer-x", "2026-05-01T00:00:00.000Z"])
    # A chunk at a LATER timestamp — must NOT advance the cursor (chunks aren't sync'd).
    raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
      ["chunk", "_chunk", "g-c", "peer-x", "2026-05-02T00:00:00.000Z"])

    cursor = @db.max_origin_cursor("peer-x")

    assert_equal "g-p", cursor["global_id"], "chunk timestamp must not advance the cursor"
  end

  def test_upsert_inserts_new_record
    rec = sync_record("g-new", origin: "peer-x", content: "hi")

    assert_equal :inserted, @db.upsert_synced(rec)
    row = @db.instance_variable_get(:@db).get_first_row("SELECT * FROM memories WHERE global_id = ?", "g-new")

    assert_equal "hi", row["content"]
    assert_equal "peer-x", row["origin_peer"]
  end

  def test_upsert_first_write_wins_for_content
    first = sync_record("g-dup", origin: "peer-x", content: "first", created_at: "2026-05-01T00:00:00.000Z")
    second = sync_record("g-dup", origin: "peer-y", content: "second", created_at: "2026-05-02T00:00:00.000Z")

    assert_equal :inserted, @db.upsert_synced(first)
    assert_equal :no_change, @db.upsert_synced(second)
    row = @db.instance_variable_get(:@db).get_first_row("SELECT content, origin_peer FROM memories WHERE global_id = ?", "g-dup")

    assert_equal "first", row["content"], "content must not change"
    assert_equal "peer-x", row["origin_peer"], "origin must not change"
  end

  def test_upsert_applies_tombstone_to_existing_row
    @db.upsert_synced(sync_record("g-x", content: "x"))
    tomb = sync_record("g-x", content: "x", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-z")

    assert_equal :updated, @db.upsert_synced(tomb)
    row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at, deleted_by_peer FROM memories WHERE global_id = ?", "g-x")

    refute_nil row["deleted_at"]
    assert_equal "peer-z", row["deleted_by_peer"]
  end

  def test_upsert_first_tombstone_wins
    @db.upsert_synced(sync_record("g-y", content: "y"))
    @db.upsert_synced(sync_record("g-y", content: "y", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-a"))
    @db.upsert_synced(sync_record("g-y", content: "y", deleted_at: "2026-05-03T00:00:00.000Z", deleted_by_peer: "peer-b"))
    row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at, deleted_by_peer FROM memories WHERE global_id = ?", "g-y")

    assert_equal "2026-05-02T00:00:00.000Z", row["deleted_at"]
    assert_equal "peer-a", row["deleted_by_peer"]
  end

  def test_upsert_tombstone_before_content_keeps_tombstone
    tomb = sync_record("g-z", origin: "peer-x", content: "", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-x")
    @db.upsert_synced(tomb)
    late = sync_record("g-z", origin: "peer-x", content: "late", created_at: "2026-05-01T00:00:00.000Z")
    @db.upsert_synced(late)
    row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at FROM memories WHERE global_id = ?", "g-z")

    refute_nil row["deleted_at"], "tombstone must persist even after late content arrives"
  end

  private

  def sync_record(global_id, origin: "peer-x", content: "data", memory_type: "note", tags: nil, metadata: nil,
    created_at: "2026-05-15T00:00:00.000Z", deleted_at: nil, deleted_by_peer: nil)
    {
      "global_id" => global_id, "origin_peer" => origin, "content" => content, "memory_type" => memory_type,
      "tags" => tags && JSON.generate(tags), "metadata" => metadata && JSON.generate(metadata),
      "created_at" => created_at, "deleted_at" => deleted_at, "deleted_by_peer" => deleted_by_peer
    }
  end
end
