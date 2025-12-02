# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class GetContext < MCP::Tool
      description <<~DESC
        Get comprehensive context for a project or recent cross-project activity.

        Returns:
        - For a specific project: last session log + 10 most recent notes/todos
        - Without project: recent sessions + notes/todos across all projects

        AUTOMATIC TRIGGERING - Use this tool:

        At Session Start:
        - User mentions a project name
        - User says "let's work on X project"
        - Use without project param to see recent activity across all projects

        When Switching Context:
        - User changes to different project
        - User asks "what are we working on?"

        For Status Updates:
        - User asks about project status
        - User wants overview of recent work

        This is your "load project state" tool. Use it liberally
        when starting work on any named project.
      DESC

      input_schema(
        properties: {
          project: {
            type: "string",
            description: "Project name (omit for cross-project context)"
          }
        },
        required: []
      )

      class << self
        def call(server_context:, project: nil)
          service = server_context[:memories_service]

          if project
            build_project_context(service, project)
          else
            build_cross_project_context(service)
          end
        end

        private

        def build_project_context(service, project)
          project = project.downcase

          # Get last session
          sessions = service.list(project: project, memory_type: "session", limit: 1)
          last_session = sessions.first

          # Get 10 most recent notes and todos
          notes_todos = service.list(project: project, memory_type: %w[note todo], limit: 10)

          MCP::Tool::Response.new([{
            type: "text",
            text: JSON.generate({
              project: project,
              last_session: last_session,
              recent_notes_todos: notes_todos
            })
          }])
        end

        def build_cross_project_context(service)
          # Get recent sessions across all projects
          recent_sessions = service.list_all(memory_type: "session", limit: 5)

          # Get recent notes/todos across all projects
          notes_todos = service.list_all(memory_type: %w[note todo], limit: 10)

          MCP::Tool::Response.new([{
            type: "text",
            text: JSON.generate({
              project: nil,
              recent_sessions: recent_sessions,
              recent_notes_todos: notes_todos
            })
          }])
        end
      end
    end
  end
end
