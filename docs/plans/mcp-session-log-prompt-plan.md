# Plan: Implement session-log as MCP Prompt

## Goal
Expose the session-log command as an MCP prompt so Claude Code can use it via the MCP protocol.

## Files to Modify/Create

| File | Action |
|------|--------|
| `lib/recollect/prompts/session_log.rb` | Create |
| `lib/recollect/mcp_server.rb` | Modify |
| `lib/recollect.rb` | Modify |
| `test/recollect/prompts/session_log_test.rb` | Create |
| `test/integration/mcp_test.rb` | Modify |

---

## Task 1: Create prompts directory and session_log prompt

Create `lib/recollect/prompts/session_log.rb`:

```ruby
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

            3. Store the summary using the memory tool:
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
```

---

## Task 2: Update lib/recollect.rb

Add require for prompts after tools:

```ruby
# After: require_relative "recollect/tools/delete_memory"
require_relative "recollect/prompts/session_log"
```

---

## Task 3: Update lib/recollect/mcp_server.rb

Add `PROMPTS` array and pass to `MCP::Server.new`:

```ruby
# frozen_string_literal: true

require "mcp"

module Recollect
  module MCPServer
    TOOLS = [
      Tools::StoreMemory,
      Tools::SearchMemory,
      Tools::GetContext,
      Tools::ListProjects,
      Tools::DeleteMemory
    ].freeze

    PROMPTS = [
      Prompts::SessionLog
    ].freeze

    class << self
      def build(db_manager)
        memories_service = MemoriesService.new(db_manager)
        MCP::Server.new(
          name: "recollect",
          version: Recollect::VERSION,
          tools: TOOLS,
          prompts: PROMPTS,
          server_context: {db_manager: db_manager, memories_service: memories_service}
        )
      end
    end
  end
end
```

---

## Task 4: Create unit test

Create `test/recollect/prompts/session_log_test.rb`:

```ruby
# frozen_string_literal: true

require "test_helper"

class SessionLogPromptTest < Recollect::TestCase
  def test_prompt_metadata
    assert_equal "session_log", Recollect::Prompts::SessionLog.name_value
    assert_match(/session summary/, Recollect::Prompts::SessionLog.description_value)
  end

  def test_prompt_arguments
    args = Recollect::Prompts::SessionLog.arguments_value
    assert_equal 1, args.size

    project_arg = args.first
    assert_equal "project", project_arg.name
    refute project_arg.required
  end

  def test_template_without_project
    result = Recollect::Prompts::SessionLog.template({}, server_context: {})

    assert_instance_of MCP::Prompt::Result, result
    assert_equal 1, result.messages.size

    message = result.messages.first
    assert_equal "user", message.role
    assert_match(/Session Log/, message.content.text)
    assert_match(/current project name/, message.content.text)
  end

  def test_template_with_project
    result = Recollect::Prompts::SessionLog.template({project: "myproject"}, server_context: {})

    message = result.messages.first
    assert_match(/"myproject"/, message.content.text)
    refute_match(/current project name/, message.content.text)
  end
end
```

---

## Task 5: Add integration tests

Add to `test/integration/mcp_test.rb`:

```ruby
def test_prompts_list
  mcp_request = {
    jsonrpc: "2.0",
    method: "prompts/list",
    id: 10
  }

  post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

  assert_predicate last_response, :ok?

  mcp_response = JSON.parse(last_response.body)
  prompts = mcp_response["result"]["prompts"]

  assert prompts.any? { |p| p["name"] == "session_log" }
end

def test_prompts_get_session_log
  mcp_request = {
    jsonrpc: "2.0",
    method: "prompts/get",
    params: {
      name: "session_log",
      arguments: {}
    },
    id: 11
  }

  post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

  assert_predicate last_response, :ok?

  mcp_response = JSON.parse(last_response.body)
  result = mcp_response["result"]

  assert result["messages"]
  assert_equal 1, result["messages"].size
  assert_equal "user", result["messages"].first["role"]
  assert_match(/Session Log/, result["messages"].first["content"]["text"])
end

def test_prompts_get_session_log_with_project
  mcp_request = {
    jsonrpc: "2.0",
    method: "prompts/get",
    params: {
      name: "session_log",
      arguments: {project: "test-project"}
    },
    id: 12
  }

  post "/mcp", mcp_request.to_json, "CONTENT_TYPE" => "application/json"

  assert_predicate last_response, :ok?

  mcp_response = JSON.parse(last_response.body)
  result = mcp_response["result"]

  assert_match(/"test-project"/, result["messages"].first["content"]["text"])
end
```

---

## Verification

```bash
bundle exec rake test
bundle exec rubocop
```

Manual test:
```bash
# List prompts
curl -s -X POST http://localhost:7326/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"prompts/list","id":1}' | jq

# Get session_log prompt
curl -s -X POST http://localhost:7326/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"prompts/get","params":{"name":"session_log","arguments":{}},"id":2}' | jq
```
