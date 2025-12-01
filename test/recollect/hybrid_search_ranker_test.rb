# frozen_string_literal: true

require "test_helper"

class HybridSearchRankerTest < Recollect::TestCase
  # Test merge with FTS-only results
  def test_merge_fts_only
    fts_results = [
      {"id" => 1, "content" => "best match", "rank" => -10.0},
      {"id" => 2, "content" => "good match", "rank" => -5.0},
      {"id" => 3, "content" => "weak match", "rank" => -1.0}
    ]
    vec_results = []

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 3, results.length
    assert_equal 1, results.first["id"], "Best FTS match should be first"
    assert_equal 3, results.last["id"], "Weakest FTS match should be last"
    assert_operator results.first["combined_score"], :>, results.last["combined_score"]
  end

  # Test merge with vector-only results
  def test_merge_vector_only
    fts_results = []
    vec_results = [
      {"id" => 1, "content" => "closest", "distance" => 0.1},
      {"id" => 2, "content" => "medium", "distance" => 0.5},
      {"id" => 3, "content" => "farthest", "distance" => 1.0}
    ]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 3, results.length
    assert_equal 1, results.first["id"], "Closest vector match should be first"
    assert_equal 3, results.last["id"], "Farthest vector match should be last"
    assert_operator results.first["combined_score"], :>, results.last["combined_score"]
  end

  # Test that dual presence boosts ranking
  def test_merge_dual_presence_wins
    fts_results = [
      {"id" => 1, "content" => "dual presence", "rank" => -8.0},
      {"id" => 2, "content" => "fts only", "rank" => -5.0}
    ]
    vec_results = [
      {"id" => 1, "content" => "dual presence", "distance" => 0.2},
      {"id" => 3, "content" => "vec only", "distance" => 0.3}
    ]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 1, results.first["id"], "Item with dual presence and good scores should win"
    assert_in_delta 0.73, results.first["combined_score"], 0.05
  end

  # Test limit is respected
  def test_merge_respects_limit
    fts_results = 5.times.map { |i| {"id" => i, "content" => "item #{i}", "rank" => -(i + 1).to_f} }
    vec_results = []

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 2)

    assert_equal 2, results.length
  end

  # Test empty inputs
  def test_merge_handles_empty_inputs
    results = Recollect::HybridSearchRanker.merge([], [], limit: 10)

    assert_empty results
  end

  # Test zero/nil values don't cause errors
  def test_merge_handles_zero_values
    fts_results = [{"id" => 1, "content" => "test", "rank" => 0}]
    vec_results = [{"id" => 2, "content" => "test2", "distance" => 0}]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 2, results.length
  end

  # Test 60/40 weighting between FTS and vector scores
  def test_merge_weighting
    fts_results = [{"id" => 1, "content" => "test", "rank" => -1.0}]
    vec_results = [{"id" => 1, "content" => "test", "distance" => 0.0}]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    # FTS: rank -1 normalized to 1.0 (only item), * 0.6 = 0.6
    # Vec: distance 0 normalized to 1.0 (best possible), * 0.4 = 0.4
    # Total: 1.0
    assert_equal 1, results.length
    assert_in_delta 1.0, results.first["combined_score"], 0.01
  end
end
