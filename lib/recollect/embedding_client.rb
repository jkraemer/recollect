# frozen_string_literal: true

require "json"
require "open3"
require "timeout"

module Recollect
  class EmbeddingClient
    class EmbeddingError < StandardError; end

    TIMEOUT = 60 # seconds (first call loads model)

    def initialize(config: Recollect.config)
      @python_path = config.python_path
      @script_path = config.embed_script_path.to_s
    end

    def embed(text)
      embed_batch([text]).first
    end

    def embed_batch(texts)
      return [] if texts.empty?

      input = JSON.generate({ texts: texts })

      output, error, status = Timeout.timeout(TIMEOUT) do
        Open3.capture3(@python_path, @script_path, stdin_data: input)
      end

      raise EmbeddingError, "Embedding script failed (exit #{status.exitstatus}): #{error}" unless status.success?

      result = JSON.parse(output)
      result["embeddings"]
    rescue JSON::ParserError => e
      raise EmbeddingError, "Invalid embedding output: #{e.message}"
    rescue Timeout::Error
      raise EmbeddingError, "Embedding script timed out after #{TIMEOUT}s"
    end

    def healthy?
      embed_batch(["test"])
      true
    rescue EmbeddingError
      false
    end
  end
end
