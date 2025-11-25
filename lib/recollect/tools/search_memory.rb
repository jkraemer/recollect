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
            type: "string",
            description: "Search query for full-text search"
          },
          project: {
            type: "string",
            description: "Limit search to specific project (omit to search all)"
          },
          memory_type: {
            type: "string",
            enum: %w[note todo],
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
          }
        },
        required: ["query"]
      )

      class << self
        # rubocop:disable Metrics/ParameterLists
        def call(query:, server_context:, project: nil, memory_type: nil, tags: nil, limit: 10)
          db_manager = server_context[:db_manager]
          search_params = { project: project, memory_type: memory_type, limit: limit }
          results = perform_search(db_manager, query, tags, search_params)

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

        def perform_search(db_manager, query, tags, params)
          if tags && !tags.empty?
            # Tag search doesn't use vectors (yet)
            db_manager.search_by_tags(tags, **params)
          else
            # Use hybrid search (auto-falls back to FTS5 if vectors unavailable)
            db_manager.hybrid_search(query, **params)
          end
        end
      end
    end
  end
end
