# frozen_string_literal: true

require "mcp"

module Recollect
  module Resources
    # Resource template resolving a single memory. Memory IDs are per-database,
    # so the project name is part of the URI; "global" addresses the global
    # database. The db-name guard runs before any database access because
    # DatabaseManager#get_database creates a database file for unknown names.
    module Memory
      def self.build(db_manager:, memories_service:)
        MCP::ResourceTemplate.define(
          uri_template: "recollect://project/{project}/memory/{id}",
          name: "memory",
          description: "A single memory, addressed by project (or 'global') and id",
          mime_type: "text/markdown"
        ) do |project:, id:|
          uri = "recollect://project/#{project}/memory/#{id}"
          raise MCP::Server::ResourceNotFoundError.new(uri) unless db_manager.list_db_names.include?(project)

          memory_id = id.match?(/\A\d+\z/) ? id.to_i : nil
          memory = memory_id && memories_service.get(memory_id, project: db_manager.project_for_db_name(project))
          raise MCP::Server::ResourceNotFoundError.new(uri) unless memory

          MCP::Resource::TextContents.new(
            text: MemoryMarkdown.memory(memory),
            uri: uri,
            mime_type: "text/markdown"
          )
        end
      end
    end
  end
end
