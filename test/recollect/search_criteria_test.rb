# frozen_string_literal: true

require "test_helper"

class SearchCriteriaTest < Recollect::TestCase
  # Test basic initialization with query string
  def test_initialize_with_query_string
    criteria = Recollect::SearchCriteria.new(query: "ruby programming")

    assert_equal "ruby programming", criteria.query
    assert_nil criteria.project
    assert_nil criteria.memory_type
    assert_equal 10, criteria.limit
    assert_nil criteria.created_after
    assert_nil criteria.created_before
  end

  # Test initialization with tags array
  def test_initialize_with_tags_array
    criteria = Recollect::SearchCriteria.new(query: %w[ruby testing])

    assert_equal %w[ruby testing], criteria.query
  end

  # Test initialization with all options
  def test_initialize_with_all_options
    criteria = Recollect::SearchCriteria.new(
      query: "test",
      project: "my-project",
      memory_type: "note",
      limit: 20,
      created_after: "2025-01-01",
      created_before: "2025-12-31"
    )

    assert_equal "test", criteria.query
    assert_equal "my-project", criteria.project
    assert_equal "note", criteria.memory_type
    assert_equal 20, criteria.limit
    assert_equal "2025-01-01", criteria.created_after
    assert_equal "2025-12-31", criteria.created_before
  end

  # Test default limit
  def test_default_limit
    criteria = Recollect::SearchCriteria.new(query: "test")

    assert_equal 10, criteria.limit
  end

  # Test date_opts helper returns hash for Database methods
  def test_date_opts_returns_hash
    criteria = Recollect::SearchCriteria.new(
      query: "test",
      created_after: "2025-01-01",
      created_before: "2025-06-01"
    )

    expected = {created_after: "2025-01-01", created_before: "2025-06-01"}

    assert_equal expected, criteria.date_opts
  end

  # Test date_opts with nil values
  def test_date_opts_with_nil_values
    criteria = Recollect::SearchCriteria.new(query: "test")

    expected = {created_after: nil, created_before: nil}

    assert_equal expected, criteria.date_opts
  end

  # Test query? returns true for string query
  def test_query_predicate_true_for_string
    criteria = Recollect::SearchCriteria.new(query: "test")

    assert_predicate criteria, :query?
  end

  # Test query? returns true for non-empty array
  def test_query_predicate_true_for_array
    criteria = Recollect::SearchCriteria.new(query: ["tag"])

    assert_predicate criteria, :query?
  end

  # Test query? returns false for nil
  def test_query_predicate_false_for_nil
    criteria = Recollect::SearchCriteria.new(query: nil)

    refute_predicate criteria, :query?
  end

  # Test query? returns false for empty string
  def test_query_predicate_false_for_empty_string
    criteria = Recollect::SearchCriteria.new(query: "")

    refute_predicate criteria, :query?
  end

  # Test query? returns false for empty array
  def test_query_predicate_false_for_empty_array
    criteria = Recollect::SearchCriteria.new(query: [])

    refute_predicate criteria, :query?
  end

  # Test project? predicate
  def test_project_predicate
    with_project = Recollect::SearchCriteria.new(query: "test", project: "proj")
    without_project = Recollect::SearchCriteria.new(query: "test")

    assert_predicate with_project, :project?
    refute_predicate without_project, :project?
  end

  # Test query_string joins array queries
  def test_query_string_joins_array
    criteria = Recollect::SearchCriteria.new(query: %w[ruby testing])

    assert_equal "ruby testing", criteria.query_string
  end

  # Test query_string returns string as-is
  def test_query_string_returns_string
    criteria = Recollect::SearchCriteria.new(query: "ruby testing")

    assert_equal "ruby testing", criteria.query_string
  end

  # Test for_project returns new criteria with different project
  def test_for_project_changes_project
    original = Recollect::SearchCriteria.new(
      query: "test",
      project: "original",
      memory_type: "note",
      limit: 20,
      created_after: "2025-01-01",
      created_before: "2025-12-31"
    )

    new_criteria = original.for_project("new-project")

    assert_equal "new-project", new_criteria.project
  end

  # Test for_project preserves other attributes
  def test_for_project_preserves_other_attributes
    original = Recollect::SearchCriteria.new(
      query: %w[ruby testing],
      project: "original",
      memory_type: "note",
      limit: 20,
      created_after: "2025-01-01",
      created_before: "2025-12-31"
    )

    new_criteria = original.for_project("new-project")

    assert_equal %w[ruby testing], new_criteria.query
    assert_equal "note", new_criteria.memory_type
    assert_equal 20, new_criteria.limit
    assert_equal "2025-01-01", new_criteria.created_after
    assert_equal "2025-12-31", new_criteria.created_before
  end

  # Test for_project with nil project (global)
  def test_for_project_with_nil
    original = Recollect::SearchCriteria.new(query: "test", project: "some-project")

    new_criteria = original.for_project(nil)

    assert_nil new_criteria.project
    refute_predicate new_criteria, :project?
  end

  # Test for_project returns new instance (immutable)
  def test_for_project_returns_new_instance
    original = Recollect::SearchCriteria.new(query: "test", project: "original")

    new_criteria = original.for_project("new")

    refute_same original, new_criteria
    assert_equal "original", original.project # original unchanged
  end
end
