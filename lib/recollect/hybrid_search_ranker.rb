# frozen_string_literal: true

module Recollect
  # Merges and ranks results from FTS5 and vector search using weighted scoring.
  # FTS results are weighted 60%, vector results 40%.
  class HybridSearchRanker
    FTS_WEIGHT = 0.6
    VECTOR_WEIGHT = 0.4

    def self.merge(fts_results, vec_results, limit:)
      scores = {}
      score_fts_results(fts_results, scores)
      score_vector_results(vec_results, scores)
      combine_and_sort(scores, limit)
    end

    def self.score_fts_results(fts_results, scores)
      max_rank = fts_results.map { |m| (m["rank"] || 0).abs }.max
      max_rank = 1.0 if max_rank.nil? || max_rank.zero?

      fts_results.each do |mem|
        normalized = (mem["rank"] || 0).abs / max_rank
        scores[mem["id"]] = {memory: mem, fts_score: normalized, vec_score: 0.0}
      end
    end

    def self.score_vector_results(vec_results, scores)
      max_distance = vec_results.map { |m| m["distance"] || 0 }.max
      max_distance = 1.0 if max_distance.nil? || max_distance.zero?

      vec_results.each do |mem|
        normalized = 1.0 - ((mem["distance"] || 0) / max_distance)
        if scores[mem["id"]]
          scores[mem["id"]][:vec_score] = normalized
        else
          scores[mem["id"]] = {memory: mem, fts_score: 0.0, vec_score: normalized}
        end
      end
    end

    def self.combine_and_sort(scores, limit)
      scored = scores.values.map do |entry|
        combined = (entry[:fts_score] * FTS_WEIGHT) + (entry[:vec_score] * VECTOR_WEIGHT)
        entry[:memory].merge("combined_score" => combined)
      end
      scored.sort_by { |m| -m["combined_score"] }.take(limit)
    end

    private_class_method :score_fts_results, :score_vector_results, :combine_and_sort
  end
end
