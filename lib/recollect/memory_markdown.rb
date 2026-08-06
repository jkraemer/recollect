# frozen_string_literal: true

module Recollect
  # Renders memories as markdown for MCP resource bodies. Pure functions of
  # the service layer's memory hashes - no MCP, no database access.
  module MemoryMarkdown
    class << self
      def memory(memory)
        meta = ["Project: #{memory["project"] || "global"}"]
        meta << "Tags: #{memory["tags"].join(", ")}" unless memory["tags"].to_a.empty?
        meta << "Created: #{memory["created_at"]}"
        meta << "Updated: #{memory["updated_at"]}"

        <<~MARKDOWN
          # Memory ##{memory["id"]} (#{memory["memory_type"]})

          #{memory["content"]}

          #{meta.map { |line| "- #{line}" }.join("\n")}
        MARKDOWN
      end

      def project_context(name:, last_session:, notes_todos:)
        <<~MARKDOWN
          # #{name}

          ## Last session

          #{last_session ? last_session["content"] : "No session log yet."}

          ## Recent notes and todos

          #{notes_todos_list(notes_todos)}
        MARKDOWN
      end

      private

      def notes_todos_list(notes_todos)
        return "No notes or todos yet." if notes_todos.empty?

        notes_todos.map do |memory|
          "- **#{memory["memory_type"]}** (##{memory["id"]}) #{memory["content"]}"
        end.join("\n")
      end
    end
  end
end
