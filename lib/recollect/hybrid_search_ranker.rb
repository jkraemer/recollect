# frozen_string_literal: true

module Recollect
  # Merges and ranks results from FTS5 and vector search using weighted scoring.
  # FTS results are weighted 60%, vector results 40%.
  class HybridSearchRanker
    FTS_WEIGHT = 0.6
    VECTOR_WEIGHT = 0.4
    RRF_K = 60

    def self.merge(fts_results, vec_results, limit:, recency_ranker: nil)
      # Switch to RRF merge
      results = rrf_merge(fts_results, vec_results, limit: limit * 2)

      if recency_ranker
        results = recency_ranker.apply(results, score_field: "combined_score")
      end

      results.take(limit)
    end

    def self.rrf_merge(fts_results, vec_results, limit:)
      scores = {}

      # fts_results are already sorted by rank (BM25)
      fts_results.each_with_index do |mem, idx|
        rank = idx + 1
        # Memory ids are only unique within a single project's database, so the
        # score map must be keyed by [project, id] or memories from different
        # projects sharing the same id (e.g. both id 1) collide and clobber
        # each other. fetch enforces the producer contract: every result must
        # carry its project (nil for global) before reaching the ranker.
        key = [mem.fetch("project"), mem["id"]]
        scores[key] ||= {memory: mem, score: 0.0}
        scores[key][:score] += FTS_WEIGHT * (1.0 / (RRF_K + rank))
      end

      # vec_results are already sorted by distance (ascending)
      vec_results.each_with_index do |mem, idx|
        rank = idx + 1
        key = [mem.fetch("project"), mem["id"]]
        scores[key] ||= {memory: mem, score: 0.0}
        scores[key][:score] += VECTOR_WEIGHT * (1.0 / (RRF_K + rank))
      end

      scored = scores.values.map do |entry|
        entry[:memory].merge("combined_score" => entry[:score])
      end

      scored.sort_by { |m| -m["combined_score"] }.take(limit)
    end

    private_class_method :rrf_merge
  end
end
