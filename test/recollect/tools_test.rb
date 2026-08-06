# frozen_string_literal: true

require "test_helper"

class ToolsHelperTest < Minitest::Test
  def test_json_response_carries_the_payload_as_text_and_structured_content
    response = Recollect::Tools.json_response({count: 2, project: nil})

    assert_kind_of MCP::Tool::Response, response
    refute_predicate response, :error?

    text = response.content.first[:text]

    assert_equal({"count" => 2, "project" => nil}, JSON.parse(text))
    assert_equal JSON.parse(text), response.structured_content
  end

  def test_json_response_normalizes_symbol_keys_to_strings_for_validation
    response = Recollect::Tools.json_response({results: [{id: 1}]})

    assert_equal [{"id" => 1}], response.structured_content["results"]
  end
end
