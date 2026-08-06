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
            type: "string",
            description: "Project name (omit for global)"
          }
        },
        required: ["id"]
      )

      output_schema(
        properties: {
          success: {type: "boolean"},
          id: {type: %w[integer null]}
        },
        required: %w[success id]
      )

      class << self
        def call(id:, server_context:, project: nil)
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
