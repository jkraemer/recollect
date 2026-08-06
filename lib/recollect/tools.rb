# frozen_string_literal: true

module Recollect
  module Tools
    # Builds the response every tool success path returns. Both representations
    # derive from the one payload: content text via JSON.generate, structured
    # content via JSON.parse of that same text - so the typed field and the
    # text are guaranteed to carry identical data, with string keys for
    # output-schema validation.
    def self.json_response(payload)
      text = JSON.generate(payload)
      MCP::Tool::Response.new(
        [{type: "text", text: text}],
        structured_content: JSON.parse(text)
      )
    end
  end
end
