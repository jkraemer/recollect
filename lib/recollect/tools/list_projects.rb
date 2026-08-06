# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class ListProjects < MCP::Tool
      description "List all projects that have stored memories"

      input_schema(properties: {})

      output_schema(
        properties: {
          projects: {type: "array", items: {type: "string"}},
          count: {type: "integer"}
        },
        required: %w[projects count]
      )

      class << self
        def call(server_context:)
          service = server_context[:memories_service]
          projects = service.list_projects

          Tools.json_response({
            projects: projects,
            count: projects.length
          })
        end
      end
    end
  end
end
