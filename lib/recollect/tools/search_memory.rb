# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class SearchMemory < MCP::Tool
      description <<~DESC
        Search memories using full-text search.

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
            description: "Search query"
          },
          project: {
            type: "string",
            description: "Limit search to specific project (omit to search all)"
          },
          memory_type: {
            type: "string",
            enum: %w[note decision pattern bug learning],
            description: "Filter by memory type"
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
        def call(query:, server_context:, project: nil, memory_type: nil, limit: 10)
          db_manager = server_context[:db_manager]

          results = db_manager.search_all(
            query,
            project: project,
            memory_type: memory_type,
            limit: limit
          )

          MCP::Tool::Response.new([{
                                    type: "text",
                                    text: JSON.generate({
                                                          results: results,
                                                          count: results.length,
                                                          query: query
                                                        })
                                  }])
        end
      end
    end
  end
end
