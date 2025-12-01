# frozen_string_literal: true

require "pathname"
require "json"

module Recollect
  class Config
    attr_accessor :data_dir, :host, :port, :max_results,
      :enable_vectors, :vector_dimensions, :embed_server_script_path,
      :log_wiredumps

    VECTOR_DIMENSIONS = 384 # all-MiniLM-L6-v2
    TRUTHY_VALUES = %w[true 1 yes on].freeze

    def initialize
      @data_dir = Pathname.new(ENV.fetch("RECOLLECT_DATA_DIR",
        File.join(Dir.home, ".recollect")))
      @host = ENV.fetch("RECOLLECT_HOST", "127.0.0.1")
      @port = ENV.fetch("RECOLLECT_PORT", "7326").to_i
      @max_results = 100

      # Vector search configuration
      @enable_vectors = env_truthy?("ENABLE_VECTORS")
      @vector_dimensions = VECTOR_DIMENSIONS
      @embed_server_script_path = Recollect.root.join("bin", "embed-server")

      # Debug logging
      @log_wiredumps = env_truthy?("LOG_WIREDUMPS")

      ensure_directories!
    end

    alias_method :log_wiredumps?, :log_wiredumps
    alias_method :enable_vectors?, :enable_vectors

    def global_db_path
      data_dir.join("global.db")
    end

    def projects_dir
      data_dir.join("projects")
    end

    def vec_extension_path
      paths = [
        "/usr/lib/vec0.so",                        # Arch Linux package
        "~/.local/lib/sqlite-vec/vec0.so",         # User local install
        "~/.local/lib/sqlite-vec/vec0.dylib",      # macOS user local
        "/usr/local/lib/vec0.so",                  # System local install
        "/usr/local/lib/vec0.dylib"                # macOS system local
      ]

      paths.each do |path|
        expanded = File.expand_path(path)
        return expanded if File.exist?(expanded)
      end

      nil
    end

    def vectors_available?
      enable_vectors? && vec_extension_path && File.executable?(embed_server_script_path)
    end

    def vector_status_message
      if vectors_available?
        "Vector embeddings: enabled"
      else
        reason = if !enable_vectors?
          "ENABLE_VECTORS not set"
        elsif !vec_extension_path
          "sqlite-vec extension not found"
        elsif !File.executable?(embed_server_script_path)
          "embed script not executable"
        else
          "unknown reason"
        end
        "Vector embeddings: disabled (#{reason})"
      end
    end

    def python_path
      # Use the venv Python if available
      venv_python = Recollect.root.join(".venv", "bin", "python3")
      return venv_python.to_s if venv_python.executable?

      # Fall back to system Python
      "python3"
    end

    def url
      "http://#{host}:#{port}"
    end

    private

    def env_truthy?(name)
      TRUTHY_VALUES.include?(ENV.fetch(name, "").downcase)
    end

    def ensure_directories!
      data_dir.mkpath
      projects_dir.mkpath
    end
  end
end
