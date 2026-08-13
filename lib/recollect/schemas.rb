# frozen_string_literal: true

module Recollect
  # JSON Schema fragments shared by the MCP tools' output schemas.
  module Schemas
    # One stored memory as the service layer returns it. memory_type is
    # deliberately not an enum: stored data contains free-form types
    # (e.g. "decision" via the CLI). rank appears only in search results.
    MEMORY = {
      type: "object",
      properties: {
        id: {type: "integer"},
        content: {type: "string"},
        memory_type: {type: "string"},
        tags: {type: "array", items: {type: "string"}},
        project: {type: %w[string null]},
        created_at: {type: "string"},
        updated_at: {type: "string"},
        rank: {type: "number"}
      },
      required: %w[id content memory_type tags project created_at updated_at]
    }.freeze
  end
end
