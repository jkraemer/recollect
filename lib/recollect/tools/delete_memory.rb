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

      class << self
        def call(id:, server_context:, project: nil)
          service = server_context[:memories_service]

          success = service.delete(id, project: project)

          MCP::Tool::Response.new([{
                                    type: "text",
                                    text: JSON.generate({
                                                          success: success,
                                                          deleted_id: success ? id : nil,
                                                          message: success ? "Memory deleted" : "Memory not found"
                                                        })
                                  }])
        end
      end
    end
  end
end
