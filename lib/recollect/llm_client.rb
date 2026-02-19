# frozen_string_literal: true

require "faraday"
require "faraday/retry"
require "json"

module Recollect
  module LlmClient
    def self.build(config = Recollect.config)
      case config.llm_provider
      when "anthropic"
        Anthropic.new(api_key: config.anthropic_api_key, model: config.anthropic_model)
      else
        None.new
      end
    end

    class Base
      def expand_query(query)
        # Default: no expansion
        [query]
      end

      def rerank(query, candidates, limit: 10)
        # Default: return as is
        candidates.take(limit)
      end

      def available?
        false
      end
    end

    class None < Base; end

    class Anthropic < Base
      def initialize(api_key:, model:)
        @api_key = api_key
        @model = model
        @conn = Faraday.new(url: "https://api.anthropic.com") do |f|
          f.request :json
          f.request :retry, max: 2, interval: 0.05, backoff_factor: 2
          f.response :json
          f.adapter Faraday.default_adapter
          f.headers["x-api-key"] = @api_key
          f.headers["anthropic-version"] = "2023-06-01"
          f.headers["content-type"] = "application/json"
        end
      end

      def available?
        !@api_key.nil? && !@api_key.empty?
      end

      def expand_query(query)
        return [query] unless available?

        prompt = <<~PROMPT
          You are a search assistant. Given a user query, provide 2 alternative search queries that use different wording or related concepts to help find relevant documents in a personal memory store.
          Output ONLY the 2 alternative queries, one per line. Do not include numbering or explanations.

          Original query: #{query}
        PROMPT

        response = @conn.post("/v1/messages", {
          model: @model,
          max_tokens: 100,
          messages: [{role: "user", content: prompt}]
        })

        if response.success?
          text = response.body.dig("content", 0, "text") || ""
          expanded = text.split("
").map(&:strip).reject(&:empty?).take(2)
          [query] + expanded
        else
          warn "[LLMClient] Anthropic query expansion failed: #{response.status} #{response.body}"
          [query]
        end
      rescue => e
        warn "[LLMClient] Error expanding query: #{e.message}"
        [query]
      end

      def rerank(query, candidates, limit: 10)
        return candidates.take(limit) if !available? || candidates.empty?

        # Prepare candidates for re-ranking
        # We only pass ID and a snippet of content to save tokens
        candidate_list = candidates.each_with_index.map do |c, i|
          "[#{i}] #{c["content"].to_s[0..500]}"
        end.join("

")

        prompt = <<~PROMPT
          Rank the following search results by their relevance to the user query: "#{query}"
          Return a comma-separated list of indices (e.g., 2,0,5,1) from most relevant to least relevant.
          Only return the indices, nothing else.

          Results:
          #{candidate_list}
        PROMPT

        response = @conn.post("/v1/messages", {
          model: @model,
          max_tokens: 100,
          messages: [{role: "user", content: prompt}]
        })

        if response.success?
          text = response.body.dig("content", 0, "text") || ""
          indices = text.scan(/\d+/).map(&:to_i)
          
          # Reorder candidates based on LLM output
          ranked = indices.map { |i| candidates[i] }.compact
          # Append any missing candidates at the end
          (ranked + (candidates - ranked)).take(limit)
        else
          warn "[LLMClient] Anthropic re-ranking failed: #{response.status} #{response.body}"
          candidates.take(limit)
        end
      rescue => e
        warn "[LLMClient] Error re-ranking: #{e.message}"
        candidates.take(limit)
      end
    end
  end
end
