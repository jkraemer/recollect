# frozen_string_literal: true

require "pathname"
require "json"

module Recollect
  class Config
    attr_accessor :data_dir, :host, :port, :max_results,
                  :enable_vectors, :vector_dimensions, :embed_server_script_path

    VECTOR_DIMENSIONS = 384 # all-MiniLM-L6-v2

    def initialize
      @data_dir = Pathname.new(ENV.fetch("RECOLLECT_DATA_DIR",
                                         File.join(Dir.home, ".recollect")))
      @host = ENV.fetch("RECOLLECT_HOST", "127.0.0.1")
      @port = ENV.fetch("RECOLLECT_PORT", "7326").to_i
      @max_results = 100

      # Vector search configuration
      @enable_vectors = ENV.fetch("ENABLE_VECTORS", "false") == "true"
      @vector_dimensions = VECTOR_DIMENSIONS
      @embed_server_script_path = Recollect.root.join("bin", "embed-server")

      ensure_directories!
    end

    def global_db_path
      data_dir.join("global.db")
    end

    def projects_dir
      data_dir.join("projects")
    end

    def project_db_path(project_name)
      projects_dir.join("#{sanitize_name(project_name)}.db")
    end

    def detect_project(cwd = Dir.pwd)
      path = Pathname.new(cwd)

      # Check for .git
      return git_remote_name(path) || path.basename.to_s if (path / ".git").exist?

      # Check for package.json
      if (path / "package.json").exist?
        data = JSON.parse((path / "package.json").read)
        return data["name"] if data["name"]
      end

      # Check for *.gemspec
      gemspec = Dir.glob(path / "*.gemspec").first
      return File.basename(gemspec, ".gemspec") if gemspec

      # Fallback to directory name (unless generic)
      name = path.basename.to_s
      return nil if %w[home Documents Desktop Downloads src code].include?(name)

      name
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
      enable_vectors && vec_extension_path && File.executable?(embed_server_script_path)
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

    def ensure_directories!
      data_dir.mkpath
      projects_dir.mkpath
    end

    def sanitize_name(name)
      name.to_s.gsub(/[^a-zA-Z0-9_]/, "_").downcase
    end

    def git_remote_name(path)
      output = `git -C "#{path}" config --get remote.origin.url 2>/dev/null`.strip
      return nil if output.empty?

      # Extract repo name from URL
      output.split("/").last&.sub(/\.git$/, "")
    rescue StandardError
      nil
    end
  end
end
