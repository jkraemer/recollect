# frozen_string_literal: true

require "mcp"

module Recollect
  module Prompts
    class ResumeSession < MCP::Prompt
      description "Resume work on a project using the last session log and recent memories"

      arguments [
        MCP::Prompt::Argument.new(
          name: "project",
          description: "Project name to resume (omit for global/cross-project sessions)",
          required: false
        )
      ]

      class << self
        def template(args, server_context:)
          project = args[:project]&.downcase
          memories_service = server_context[:memories_service]

          last_session = fetch_last_session(memories_service, project)
          recent_memories = fetch_recent_memories(memories_service, project)

          prompt_text = build_prompt(project, last_session, recent_memories)

          MCP::Prompt::Result.new(
            messages: [
              MCP::Prompt::Message.new(
                role: "user",
                content: MCP::Content::Text.new(prompt_text)
              )
            ]
          )
        end

        private

        def fetch_last_session(memories_service, project)
          sessions = memories_service.list(project: project, memory_type: "session", limit: 1)
          sessions.first
        end

        def fetch_recent_memories(memories_service, project)
          # Get recent non-session memories for additional context
          memories = memories_service.list(project: project, memory_type: nil, limit: 20)
          memories.reject { |m| m["memory_type"] == "session" }.take(10)
        end

        def build_prompt(project, last_session, recent_memories)
          project_label = project || "global"

          parts = ["# Resume Session\n"]
          parts << "Project: #{project_label}\n\n"

          parts << "## Last Session Log\n\n"
          if last_session
            parts << last_session["content"]
            parts << "\n\n"
            if last_session["created_at"]
              parts << "_Recorded: #{last_session["created_at"]}_\n\n"
            end
          else
            parts << "_No previous session log found for this project._\n\n"
          end

          if recent_memories.any?
            parts << "## Recent Memories\n\n"
            recent_memories.each do |memory|
              parts << format_memory(memory)
              parts << "\n"
            end
            parts << "\n"
          end

          parts << "## Instructions\n\n"
          parts << <<~INSTRUCTIONS
            Review the session log and recent memories above to understand the context of previous work.

            Based on this context:
            1. Summarize what was being worked on
            2. Identify any incomplete tasks or next steps mentioned
            3. Ask if the user wants to continue with the suggested next steps or work on something else
          INSTRUCTIONS

          parts.join
        end

        def format_memory(memory)
          tags = memory["tags"]&.any? ? " [#{memory["tags"].join(", ")}]" : ""
          type = memory["memory_type"] || "note"
          date = memory["created_at"] ? " (#{memory["created_at"]})" : ""

          "- **#{type}**#{tags}#{date}: #{memory["content"].lines.first&.strip || "(empty)"}"
        end
      end
    end
  end
end
