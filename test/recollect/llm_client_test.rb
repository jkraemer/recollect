# frozen_string_literal: true

require "test_helper"

class LlmClientTest < Recollect::TestCase
  def test_build_none_provider
    config = Struct.new(:llm_provider).new("none")
    client = Recollect::LlmClient.build(config)
    
    assert_instance_of Recollect::LlmClient::None, client
    assert_equal ["query"], client.expand_query("query")
    assert_equal [1, 2], client.rerank("query", [1, 2], limit: 10)
    refute client.available?
  end

  def test_anthropic_available_check
    client = Recollect::LlmClient::Anthropic.new(api_key: nil, model: "haiku")
    refute client.available?

    client = Recollect::LlmClient::Anthropic.new(api_key: "valid", model: "haiku")
    assert client.available?
  end

  # We mock the Faraday connection to avoid real API calls
  def test_anthropic_expand_query
    mock_conn = Minitest::Mock.new
    mock_response = Minitest::Mock.new
    
    mock_response.expect :success?, true
    mock_response.expect :body, { "content" => [{ "text" => "alternative 1
alternative 2" }] }
    
    mock_conn.expect :post, mock_response, ["/v1/messages", Hash]
    
    client = Recollect::LlmClient::Anthropic.new(api_key: "test", model: "haiku")
    # Inject mock connection
    client.instance_variable_set(:@conn, mock_conn)
    
    expanded = client.expand_query("original")
    
    assert_equal ["original", "alternative 1", "alternative 2"], expanded
    mock_conn.verify
  end

  def test_anthropic_rerank
    mock_conn = Minitest::Mock.new
    mock_response = Minitest::Mock.new
    
    mock_response.expect :success?, true
    mock_response.expect :body, { "content" => [{ "text" => "1, 0" }] }
    
    mock_conn.expect :post, mock_response, ["/v1/messages", Hash]
    
    client = Recollect::LlmClient::Anthropic.new(api_key: "test", model: "haiku")
    client.instance_variable_set(:@conn, mock_conn)
    
    candidates = [
      { "id" => 10, "content" => "first content" },
      { "id" => 20, "content" => "second content" }
    ]
    
    ranked = client.rerank("query", candidates, limit: 2)
    
    assert_equal 20, ranked.first["id"], "Should be second candidate based on LLM ranking"
    assert_equal 10, ranked.last["id"]
    mock_conn.verify
  end
end
