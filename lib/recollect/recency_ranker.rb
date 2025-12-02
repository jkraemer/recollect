# frozen_string_literal: true

require "time"

module Recollect
  # Applies recency-based score adjustment to search results.
  # Uses exponential decay with configurable half-life.
  #
  # Formula:
  #   effective_recency = 1 - aging_factor + (aging_factor * exp_decay)
  #   exp_decay = e^(-lambda * age_in_days)
  #   lambda = ln(2) / half_life_days
  #
  # When aging_factor=0, no recency effect.
  # When aging_factor=1, full recency effect.
  class RecencyRanker
    LN_2 = Math.log(2)
    SECONDS_PER_DAY = 86400.0

    def initialize(aging_factor:, half_life_days:, reference_time: nil)
      @aging_factor = aging_factor.to_f.clamp(0.0, 1.0)
      @half_life_days = [half_life_days.to_f, 0.1].max # Avoid division by zero
      @lambda = LN_2 / @half_life_days
      @reference_time = reference_time || Time.now
    end

    # Calculate recency factor for a single timestamp.
    # Returns value between (1 - aging_factor) and 1.0
    #
    # @param created_at [String, Time, nil] ISO 8601 timestamp or Time object
    # @return [Float] Recency factor
    def calculate_recency_factor(created_at)
      return 1.0 if @aging_factor.zero?
      return 1.0 if created_at.nil?

      timestamp = parse_timestamp(created_at)
      return 1.0 if timestamp.nil?

      age_in_days = (@reference_time - timestamp) / SECONDS_PER_DAY
      age_in_days = [age_in_days, 0.0].max # Handle future timestamps

      exp_decay = Math.exp(-@lambda * age_in_days)
      1 - @aging_factor + (@aging_factor * exp_decay)
    end

    # Apply recency ranking to results.
    # Results must have a score field and created_at timestamp.
    #
    # @param results [Array<Hash>] Search results with score and created_at
    # @param score_field [String] Name of the score field to modify
    # @return [Array<Hash>] Re-sorted results with adjusted scores
    def apply(results, score_field:)
      return results if @aging_factor.zero? || results.empty?

      results.map do |result|
        recency_factor = calculate_recency_factor(result["created_at"])
        original_score = result[score_field] || 0.0
        adjusted_score = original_score * recency_factor

        result.merge(
          score_field => adjusted_score,
          "recency_factor" => recency_factor
        )
      end.sort_by { |r| -(r[score_field] || 0.0) }
    end

    private

    def parse_timestamp(value)
      case value
      when Time
        value
      when String
        Time.parse(value)
      end
    rescue ArgumentError
      nil
    end
  end
end
