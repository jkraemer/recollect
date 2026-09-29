# frozen_string_literal: true

require "test_helper"

class HybridSearchRankerTest < Recollect::TestCase
  # Test merge with FTS-only results
  def test_merge_fts_only
    fts_results = [
      {"id" => 1, "project" => nil, "content" => "best match", "rank" => -10.0},
      {"id" => 2, "project" => nil, "content" => "good match", "rank" => -5.0},
      {"id" => 3, "project" => nil, "content" => "weak match", "rank" => -1.0}
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
      {"id" => 1, "project" => nil, "content" => "closest", "distance" => 0.1},
      {"id" => 2, "project" => nil, "content" => "medium", "distance" => 0.5},
      {"id" => 3, "project" => nil, "content" => "farthest", "distance" => 1.0}
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
      {"id" => 1, "project" => nil, "content" => "dual presence", "rank" => -8.0},
      {"id" => 2, "project" => nil, "content" => "fts only", "rank" => -5.0}
    ]
    vec_results = [
      {"id" => 1, "project" => nil, "content" => "dual presence", "distance" => 0.2},
      {"id" => 3, "project" => nil, "content" => "vec only", "distance" => 0.3}
    ]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 1, results.first["id"], "Item with dual presence and good scores should win"
    # FTS: id 1 is rank 1 -> 0.6 * (1/61) = 0.009836
    # Vec: id 1 is rank 1 -> 0.4 * (1/61) = 0.006557
    # Total: 0.016393
    assert_in_delta 0.016393, results.first["combined_score"], 0.001
  end

  # Test limit is respected
  def test_merge_respects_limit
    fts_results = 5.times.map { |i| {"id" => i, "project" => nil, "content" => "item #{i}", "rank" => -(i + 1).to_f} }
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
    fts_results = [{"id" => 1, "project" => nil, "content" => "test", "rank" => 0}]
    vec_results = [{"id" => 2, "project" => nil, "content" => "test2", "distance" => 0}]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 2, results.length
  end

  # Test 60/40 weighting between FTS and vector scores
  def test_merge_weighting
    fts_results = [{"id" => 1, "project" => nil, "content" => "test", "rank" => -1.0}]
    vec_results = [{"id" => 1, "project" => nil, "content" => "test", "distance" => 0.0}]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    # RRF rank 1 for both: 0.6 * (1/61) + 0.4 * (1/61) = 1.0 / 61 = 0.016393
    assert_equal 1, results.length
    assert_in_delta 0.016393, results.first["combined_score"], 0.001
  end

  # rrf_merge keys its score map by [project, id]; a result lacking the key
  # would silently reintroduce cross-project id collisions. Producers must
  # stamp project (nil for global) before results reach the ranker.
  def test_merge_rejects_results_missing_project
    fts_results = [{"id" => 1, "content" => "no project key", "rank" => -1.0}]

    assert_raises(KeyError) do
      Recollect::HybridSearchRanker.merge(fts_results, [], limit: 10)
    end
  end

  def test_merge_rejects_vector_results_missing_project
    vec_results = [{"id" => 1, "content" => "no project key", "distance" => 0.1}]

    assert_raises(KeyError) do
      Recollect::HybridSearchRanker.merge([], vec_results, limit: 10)
    end
  end

  # Test that memories from different projects sharing the same numeric id
  # (per-database AUTOINCREMENT ids collide across project databases) are
  # kept as distinct entries instead of one clobbering the other.
  def test_merge_keeps_colliding_ids_from_different_projects_distinct
    fts_results = [
      {"id" => 1, "project" => "project-a", "content" => "project a memory", "rank" => -10.0},
      {"id" => 1, "project" => "project-b", "content" => "project b memory", "rank" => -5.0}
    ]
    vec_results = []

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    assert_equal 2, results.length, "Same id in different projects must not collide"
    assert_equal ["project-a", "project-b"], results.map { |m| m["project"] }.sort
  end

  # Test that dual presence fusion still sums both contributions into one
  # entry when the FTS and vector hits are genuinely the same memory
  # (matching project AND id), not just the same bare id.
  def test_merge_dual_presence_same_project_still_sums
    fts_results = [{"id" => 1, "project" => "project-a", "content" => "test", "rank" => -1.0}]
    vec_results = [{"id" => 1, "project" => "project-a", "content" => "test", "distance" => 0.0}]

    results = Recollect::HybridSearchRanker.merge(fts_results, vec_results, limit: 10)

    # RRF rank 1 for both: 0.6 * (1/61) + 0.4 * (1/61) = 1.0 / 61 = 0.016393
    assert_equal 1, results.length
    assert_in_delta 0.016393, results.first["combined_score"], 0.001
  end

  def test_merge_with_recency_ranker
    reference_time = Time.parse("2025-01-15T12:00:00Z")
    recency_ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 7,
      reference_time: reference_time
    )

    fts_results = [
      {"id" => 1, "project" => nil, "content" => "old but relevant", "rank" => -10.0,
       "created_at" => "2025-01-01T12:00:00Z"}, # 14 days old = 2 half-lives
      {"id" => 2, "project" => nil, "content" => "recent but less relevant", "rank" => -5.0,
       "created_at" => "2025-01-14T12:00:00Z"}  # 1 day old
    ]
    vec_results = []

    results = Recollect::HybridSearchRanker.merge(
      fts_results, vec_results,
      limit: 10,
      recency_ranker: recency_ranker
    )

    # Recent item should be boosted higher despite lower initial score
    assert_equal 2, results.first["id"]
    assert results.first.key?("recency_factor")
  end

  def test_merge_without_recency_ranker_unchanged
    fts_results = [
      {"id" => 1, "project" => nil, "content" => "relevant", "rank" => -10.0,
       "created_at" => "2025-01-01T12:00:00Z"},
      {"id" => 2, "project" => nil, "content" => "less relevant", "rank" => -5.0,
       "created_at" => "2025-01-14T12:00:00Z"}
    ]
    vec_results = []

    results = Recollect::HybridSearchRanker.merge(
      fts_results, vec_results,
      limit: 10
    )

    # Without recency, order by original relevance
    assert_equal 1, results.first["id"]
    refute results.first.key?("recency_factor")
  end
end
