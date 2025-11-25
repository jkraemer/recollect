# frozen_string_literal: true

require "json"
require "open3"
require "timeout"

module Recollect
  class EmbeddingClient
    class EmbeddingError < StandardError; end

    STARTUP_TIMEOUT = 120 # Model loading can take time
    REQUEST_TIMEOUT = 30  # Individual requests should be fast

    def initialize(config: Recollect.config)
      @python_path = config.python_path
      @script_path = config.embed_server_script_path.to_s
      @mutex = Mutex.new
      @stdin = nil
      @stdout = nil
      @stderr = nil
      @wait_thread = nil
    end

    def embed(text)
      embed_batch([text]).first
    end

    def embed_batch(texts)
      return [] if texts.empty?

      @mutex.synchronize do
        ensure_process_running!
        send_request({ texts: texts })
      end
    end

    def healthy?
      @mutex.synchronize do
        return false unless process_alive?

        send_request({ ping: true }, timeout: 5)
        true
      end
    rescue EmbeddingError
      false
    end

    def shutdown
      @mutex.synchronize do
        stop_process
      end
    end

    private

    def ensure_process_running!
      return if process_alive?

      start_process
    end

    def process_alive?
      @wait_thread&.alive?
    end

    def start_process
      stop_process if @wait_thread

      @stdin, @stdout, @stderr, @wait_thread = Open3.popen3(
        @python_path, @script_path
      )

      wait_for_ready
    end

    def wait_for_ready
      Timeout.timeout(STARTUP_TIMEOUT) do
        loop do
          line = @stderr.gets
          break if line.nil?
          break if line.include?("Ready for requests")
        end
      end
    rescue Timeout::Error
      stop_process
      raise EmbeddingError, "Embedding process startup timed out after #{STARTUP_TIMEOUT}s"
    end

    def stop_process
      close_streams
      terminate_process
      @stdin = @stdout = @stderr = @wait_thread = nil
    end

    def close_streams
      [@stdin, @stdout, @stderr].each { |io| io&.close rescue nil } # rubocop:disable Style/RescueModifier
    end

    def terminate_process
      return unless @wait_thread&.alive?

      Process.kill("TERM", @wait_thread.pid) rescue nil # rubocop:disable Style/RescueModifier
      @wait_thread.join(5) rescue nil # rubocop:disable Style/RescueModifier
    end

    def send_request(request, timeout: REQUEST_TIMEOUT)
      raise EmbeddingError, "Process not running" unless process_alive?

      json_line = "#{JSON.generate(request)}\n"
      @stdin.write(json_line)
      @stdin.flush

      response_line = Timeout.timeout(timeout) { @stdout.gets }

      raise EmbeddingError, "Process died (stdout closed)" if response_line.nil?

      result = JSON.parse(response_line)
      raise EmbeddingError, result["error"] if result["error"]

      result["pong"] ? true : result["embeddings"]
    rescue Timeout::Error
      stop_process
      raise EmbeddingError, "Embedding request timed out after #{timeout}s"
    rescue Errno::EPIPE
      stop_process
      raise EmbeddingError, "Embedding process died (broken pipe)"
    rescue JSON::ParserError => e
      raise EmbeddingError, "Invalid response from embedding process: #{e.message}"
    end
  end
end
