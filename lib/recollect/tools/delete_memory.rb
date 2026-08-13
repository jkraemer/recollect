# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class DeleteMemory < MCP::Tool
      description "Delete a specific memory by ID"

      input_schema(
        properties: {
          id: {
            type: "integer",
            description: "Memory ID to delete"
          },
          project: {
            type: %w[string null],
            description: "Project the memory belongs to, as returned in search results (null for a global memory)"
          }
        },
        required: %w[id project]
      )

      output_schema(
        properties: {
          success: {type: "boolean"},
          id: {type: %w[integer null]}
        },
        required: %w[success id]
      )

      class << self
        def call(id:, project:, server_context:)
          service = server_context[:memories_service]

          success = service.delete(id, project: project)

          Tools.json_response({
            success: success,
            id: success ? id : nil
          })
        end
      end
    end
  end
end
