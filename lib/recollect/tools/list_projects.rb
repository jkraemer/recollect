# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class ListProjects < MCP::Tool
      description "List all projects that have stored memories"

      input_schema(properties: {})

      class << self
        def call(server_context:)
          service = server_context[:memories_service]
          projects = service.list_projects

          MCP::Tool::Response.new([{
            type: "text",
            text: JSON.generate({
              projects: projects,
              count: projects.length
            })
          }])
        end
      end
    end
  end
end
