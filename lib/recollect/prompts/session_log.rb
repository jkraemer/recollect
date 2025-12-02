# frozen_string_literal: true

require "mcp"

module Recollect
  module Prompts
    class SessionLog < MCP::Prompt
      description "Create a session summary and store it in long-term memory for future retrieval"

      arguments [
        MCP::Prompt::Argument.new(
          name: "project",
          description: "Project name for storing the session summary",
          required: false
        )
      ]

      class << self
        def template(args, server_context:)
          project = args[:project]
          prompt_text = build_prompt(project)

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

        def build_prompt(project)
          project_instruction = if project
            "\"#{project}\""
          else
            "current project name (or omit for cross-project sessions)"
          end

          <<~PROMPT
            # Session Log

            Create a session summary and store it in long-term memory for future retrieval.

            ## Instructions

            1. Review the current conversation and identify:
               - What was worked on
               - Key decisions made
               - Problems solved
               - Current state of work
               - Logical next steps

            2. Create a structured summary following this format:

            ## Session Summary Template

            Session: [Descriptive Title]
            Date: [Current UTC timestamp]

            ### Overview
            [2-3 sentences summarizing what was accomplished]

            ### Key Decisions
            - [Decision and reasoning]

            ### Problems Solved
            - [Problem]: [Solution]

            ### Current State
            [What's working, what's partial, what's broken]

            ### Next Steps
            1. [Immediate next action]
            2. [Following action]

            ### Context for Continuation
            [Anything a future session needs to know to continue seamlessly]

            3. Store the summary using the store_memory tool:
               - memory_type: "session"
               - tags: [relevant topic tags]
               - project: #{project_instruction}

            4. Confirm storage to the user with the memory ID.
          PROMPT
        end
      end
    end
  end
end
