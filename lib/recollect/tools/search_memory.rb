# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class SearchMemory < MCP::Tool
      description <<~DESC
        Search memories using full-text search and/or tag filtering.

        SEARCH STRATEGIES:
        1. Full-text search: Use query parameter to search content
        2. Tag filtering: Use tags parameter to filter by specific tags (AND logic)
        3. Combined: Use both for precise results

        TAG-BASED SEARCHING:
        Tags now carry semantic meaning. Search by tags to find:
        - Decisions: tags=["decision"]
        - Patterns: tags=["pattern"]
        - Bugs: tags=["bug"]
        - Learnings: tags=["learning"]
        - Combined: tags=["architecture", "decision"] (finds memories with both tags)

        AUTOMATIC TRIGGERING - Use when user:

        Asks About Past Decisions:
        - "What did we decide about...?"
        - "Why did we choose...?"
        - "What was the reasoning for...?"

        References Previous Work:
        - "Last time we..."
        - "Previously, we..."
        - "Remember when we..."

        Asks Implementation Questions:
        - "How did we implement...?"
        - "What approach did we use for...?"

        Troubleshooting:
        - "Have we seen this error before?"
        - "Is there a known workaround?"

        Trigger Words (search immediately):
        - Past tense: "decided", "implemented", "discussed"
        - Time references: "yesterday", "last week", "previously"
        - Memory references: "remember", "recall", "mentioned"

        IMPORTANT: Search proactively when user references past work.
      DESC

      input_schema(
        properties: {
          query: {
            oneOf: [
              { type: "string" },
              { type: "array", items: { type: "string" } }
            ],
            description: "Search query: string for phrase search, or array of terms for AND search"
          },
          project: {
            type: "string",
            description: "Limit search to specific project (omit to search all)"
          },
          memory_type: {
            type: "string",
            enum: %w[note todo session],
            description: "Filter by memory type"
          },
          tags: {
            type: "array",
            items: { type: "string" },
            description: "Filter by tags (AND logic - memory must have all specified tags)"
          },
          limit: {
            type: "integer",
            description: "Maximum results (default: 10)",
            default: 10
          },
          created_after: {
            type: "string",
            description: "Filter to memories created on or after this date (YYYY-MM-DD)"
          },
          created_before: {
            type: "string",
            description: "Filter to memories created on or before this date (YYYY-MM-DD)"
          }
        },
        required: ["query"]
      )

      class << self
        # rubocop:disable Metrics/ParameterLists
        def call(query:, server_context:, project: nil, memory_type: nil, tags: nil, limit: 10,
                 created_after: nil, created_before: nil)
          service = server_context[:memories_service]
          tag_search = tags && !tags.empty?
          criteria = build_criteria(query, tags, project:, memory_type:, limit:, created_after:, created_before:)
          results = perform_search(service, criteria, tag_search: tag_search)

          MCP::Tool::Response.new([{
                                    type: "text",
                                    text: JSON.generate({
                                                          results: results,
                                                          count: results.length,
                                                          query: query
                                                        })
                                  }])
        end
        # rubocop:enable Metrics/ParameterLists

        private

        def build_criteria(query, tags, **)
          # Use tags as query if searching by tags, otherwise use text query
          search_query = tags && !tags.empty? ? tags : query
          SearchCriteria.new(query: search_query, **)
        end

        def perform_search(service, criteria, tag_search:)
          if tag_search
            service.search_by_tags(criteria)
          else
            service.search(criteria)
          end
        end
      end
    end
  end
end
