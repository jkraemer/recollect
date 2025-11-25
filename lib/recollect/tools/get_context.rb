# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class GetContext < MCP::Tool
      description <<~DESC
        Get comprehensive context for a project.

        AUTOMATIC TRIGGERING - Use this tool:

        At Session Start:
        - User mentions a project name
        - User says "let's work on X project"

        When Switching Context:
        - User changes to different project
        - User asks "what are we working on?"

        For Status Updates:
        - User asks about project status
        - User wants overview of decisions

        This is your "load project state" tool. Use it liberally
        when starting work on any named project.
      DESC

      input_schema(
        properties: {
          project: {
            type: "string",
            description: "Project name"
          }
        },
        required: ["project"]
      )

      class << self
        def call(project:, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          memories = db.list(limit: 100)
          by_type = memories.group_by { |m| m["memory_type"] }

          # Get recent (last 7 days)
          cutoff = (Time.now - (7 * 24 * 60 * 60)).strftime("%Y-%m-%dT%H:%M:%SZ")
          recent = memories.select { |m| m["created_at"] > cutoff }

          MCP::Tool::Response.new([{
                                    type: "text",
                                    text: JSON.generate({
                                                          project: project,
                                                          total_memories: memories.length,
                                                          recent_count: recent.length,
                                                          by_type: by_type.transform_values(&:length),
                                                          recent_memories: recent.take(20)
                                                        })
                                  }])
        end
      end
    end
  end
end
