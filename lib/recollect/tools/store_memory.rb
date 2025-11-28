# frozen_string_literal: true

require "mcp"

module Recollect
  module Tools
    class StoreMemory < MCP::Tool
      description <<~DESC
        Store a memory for later retrieval.

        MEMORY TYPES:
        - note: General information, facts, context (default)
        - todo: Action items, tasks, reminders

        TAGGING PHILOSOPHY:
        Use tags to add semantic meaning to your memories. Instead of memory types
        like "decision", "pattern", "bug", or "learning", use descriptive tags:

        Examples:
        - Architectural decision → tags: ["architecture", "decision"]
        - Bug fix → tags: ["bug", "authentication"]
        - Design pattern → tags: ["pattern", "singleton"]
        - Learning/insight → tags: ["learning", "performance"]

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
            type: "string",
            description: "The memory content to store"
          },
          memory_type: {
            type: "string",
            enum: %w[note todo],
            description: "Type of memory (default: note)"
          },
          tags: {
            type: "array",
            items: { type: "string" },
            description: "Tags for categorization (e.g., decision, pattern, bug, learning)"
          },
          project: {
            type: "string",
            description: "Project name (omit for global memory)"
          }
        },
        required: ["content"]
      )

      class << self
        def call(content:, server_context:, memory_type: "note", tags: nil, project: nil)
          service = server_context[:memories_service]

          memory = service.create(
            content: content,
            project: project,
            memory_type: memory_type,
            tags: tags || [],
            source: "mcp"
          )

          location = memory["project"] ? "project '#{memory["project"]}'" : "global"

          MCP::Tool::Response.new([{
                                    type: "text",
                                    text: JSON.generate({
                                                          success: true,
                                                          id: memory["id"],
                                                          stored_in: location,
                                                          message: "Memory stored successfully in #{location}"
                                                        })
                                  }])
        end
      end
    end
  end
end
