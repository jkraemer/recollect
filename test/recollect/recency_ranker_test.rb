# frozen_string_literal: true

require "test_helper"

class RecencyRankerTest < Recollect::TestCase
  def setup
    super
    @reference_time = Time.parse("2025-01-15T12:00:00Z")
  end

  def test_recency_factor_for_current_time
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    # Same time = factor of 1.0
    factor = ranker.calculate_recency_factor("2025-01-15T12:00:00Z")

    assert_in_delta 1.0, factor, 0.001
  end

  def test_recency_factor_at_half_life
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # 14 days ago = factor of 0.5
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")

    assert_in_delta 0.5, factor, 0.01
  end

  def test_recency_factor_at_two_half_lives
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 7,
      reference_time: @reference_time
    )

    # 14 days = 2 half-lives = factor of 0.25
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")

    assert_in_delta 0.25, factor, 0.01
  end

  def test_recency_factor_with_partial_aging
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 0.5,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # 14 days with aging_factor=0.5:
    # effective = 1 - 0.5 + (0.5 * 0.5) = 0.75
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")

    assert_in_delta 0.75, factor, 0.01
  end

  def test_recency_factor_with_zero_aging
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 0.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    # aging_factor=0 means no decay, always 1.0
    factor = ranker.calculate_recency_factor("2025-01-01T12:00:00Z")

    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_nil_timestamp
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    factor = ranker.calculate_recency_factor(nil)

    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_invalid_timestamp
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    factor = ranker.calculate_recency_factor("not-a-timestamp")

    assert_in_delta 1.0, factor, 0.001
  end

  def test_handles_time_object
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    timestamp = Time.parse("2025-01-01T12:00:00Z")
    factor = ranker.calculate_recency_factor(timestamp)

    assert_in_delta 0.5, factor, 0.01
  end

  def test_apply_returns_empty_for_empty_results
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    results = ranker.apply([], score_field: "score")

    assert_empty results
  end

  def test_apply_skips_when_aging_factor_zero
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 0.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    results = [
      {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"}
    ]

    ranked = ranker.apply(results, score_field: "score")

    assert_equal 1, ranked.first["id"]
    assert_in_delta 1.0, ranked.first["score"], 0.001
    refute ranked.first.key?("recency_factor")
  end

  def test_apply_adjusts_scores
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 14,
      reference_time: @reference_time
    )

    results = [
      {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"} # 14 days old
    ]

    ranked = ranker.apply(results, score_field: "score")

    # Score should be halved (1.0 * 0.5 = 0.5)
    assert_in_delta 0.5, ranked.first["score"], 0.01
    assert_in_delta 0.5, ranked.first["recency_factor"], 0.01
  end

  def test_apply_resorts_by_adjusted_score
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 7,
      reference_time: @reference_time
    )

    results = [
      {"id" => 1, "score" => 1.0, "created_at" => "2025-01-01T12:00:00Z"}, # Old, high score
      {"id" => 2, "score" => 0.4, "created_at" => "2025-01-15T12:00:00Z"}, # New, low score
      {"id" => 3, "score" => 0.6, "created_at" => "2025-01-08T12:00:00Z"}  # Medium
    ]

    ranked = ranker.apply(results, score_field: "score")

    # Order by adjusted score: new beats old despite lower original
    # ID 2: 0.4 * 1.0 = 0.4, ID 3: 0.6 * 0.5 = 0.3, ID 1: 1.0 * 0.25 = 0.25
    assert_equal [2, 3, 1], ranked.map { |r| r["id"] }
  end

  def test_apply_preserves_original_data
    ranker = Recollect::RecencyRanker.new(
      aging_factor: 1.0,
      half_life_days: 30,
      reference_time: @reference_time
    )

    results = [
      {"id" => 1, "score" => 1.0, "created_at" => "2025-01-10T12:00:00Z", "content" => "test"}
    ]

    ranked = ranker.apply(results, score_field: "score")

    assert_equal "test", ranked.first["content"]
    assert_equal 1, ranked.first["id"]
  end
end
