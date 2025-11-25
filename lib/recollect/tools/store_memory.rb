# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class StoreMemory < MCP::Tool
      description <<~DESC
        Store a memory for later retrieval.

        AUTOMATIC TRIGGERING - Use this tool when you observe:

        Decisions & Architecture:
        - User makes architectural decisions
        - User explains why they chose an approach
        - User discusses trade-offs between options

        Bug Solutions:
        - User describes a bug and its solution
        - User explains workarounds
        - User documents known issues

        Patterns & Conventions:
        - User establishes coding patterns
        - User defines project conventions

        Trigger Phrases (store immediately):
        - "remember that..."
        - "for future reference..."
        - "we decided..."
        - "the solution is..."

        IMPORTANT: Store proactively! Don't wait for explicit commands.
      DESC

      input_schema(
        properties: {
          content: {
            type: 'string',
            description: 'The memory content to store'
          },
          memory_type: {
            type: 'string',
            enum: %w[note decision pattern bug learning],
            description: 'Type of memory (default: note)'
          },
          tags: {
            type: 'array',
            items: { type: 'string' },
            description: 'Tags for categorization'
          },
          project: {
            type: 'string',
            description: 'Project name (omit for global memory)'
          }
        },
        required: ['content']
      )

      class << self
        def call(content:, memory_type: 'note', tags: nil, project: nil, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          id = db.store(
            content: content,
            memory_type: memory_type,
            tags: tags,
            source: 'mcp'
          )

          location = project ? "project '#{project}'" : 'global'

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              success: true,
              id: id,
              stored_in: location,
              message: "Memory stored successfully in #{location}"
            })
          }])
        end
      end
    end
  end
end
