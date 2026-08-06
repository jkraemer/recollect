# frozen_string_literal: true

require "mcp"

module Recollect
  module Resources
    # One MCP resource per database (global plus each project), listing the
    # project's memory as markdown. Built per request so a newly created
    # project is listable immediately.
    module Projects
      def self.build(db_manager:, memories_service:)
        db_manager.list_db_names.map do |name|
          project = db_manager.project_for_db_name(name)
          uri = "recollect://project/#{name}"

          MCP::Resource.define(
            uri: uri,
            name: name,
            description: "Memory for #{name}: last session log and recent notes/todos",
            mime_type: "text/markdown"
          ) do
            sessions = memories_service.list(project: project, memory_type: "session", limit: 1)
            notes_todos = memories_service.list(project: project, memory_type: %w[note todo], limit: 10)

            MCP::Resource::TextContents.new(
              text: MemoryMarkdown.project_context(
                name: name, last_session: sessions.first, notes_todos: notes_todos
              ),
              uri: uri,
              mime_type: "text/markdown"
            )
          end
        end
      end
    end
  end
end
