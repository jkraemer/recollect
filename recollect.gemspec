# frozen_string_literal: true

require_relative "lib/recollect/version"

Gem::Specification.new do |spec|
  spec.name = "recollect"
  spec.version = Recollect::VERSION
  spec.authors = ["Jens Kraemer"]
  spec.email = ["jk@jkraemer.net"]

  spec.summary = "Persistent memory server for AI coding assistants"
  spec.description = "Recollect stores decisions, patterns and learnings in per-project SQLite " \
    "databases and serves them over MCP and a REST API, with hybrid full-text and vector search."
  spec.homepage = "https://github.com/jkraemer/recollect"
  spec.license = "GPL-3.0-or-later"
  spec.required_ruby_version = ">= 3.4.0"

  spec.metadata["source_code_uri"] = spec.homepage
  spec.metadata["bug_tracker_uri"] = "#{spec.homepage}/issues"
  spec.metadata["rubygems_mfa_required"] = "true"

  # The Claude Code plugin (skills, commands, marketplace manifests) is distributed
  # through the git repository, not the gem - see README.
  spec.files = IO.popen(%w[git ls-files -z], chdir: __dir__, err: IO::NULL) { |ls| ls.read }
    .split("\x0")
    .reject do |path|
      path.start_with?("test/", "docs/", "skills/", "commands/", ".claude-plugin/") ||
        path.start_with?(".git", ".rubocop", ".mcp.json") ||
        # bin/ holds working-copy wrappers; bin/embed-server is the one the server runs
        (path.start_with?("bin/") && path != "bin/embed-server") ||
        %w[Rakefile Gemfile Gemfile.lock CLAUDE.md GEMINI.md].include?(path)
    end

  spec.bindir = "exe"
  spec.executables = spec.files.grep(%r{\Aexe/}) { |f| File.basename(f) }
  spec.require_paths = ["lib"]

  spec.add_dependency "mcp", "~> 1.1"
  spec.add_dependency "puma", "~> 7.0"
  spec.add_dependency "sinatra", "~> 4.0"
  spec.add_dependency "sqlite3", "~> 2.0"
  spec.add_dependency "faraday", "~> 2.10"
  spec.add_dependency "faraday-retry", "~> 2.2"

  spec.add_dependency "pastel", "~> 0.8"
  spec.add_dependency "thor", "~> 1.3"
  spec.add_dependency "tty-table", "~> 0.12"

  spec.add_dependency "zeitwerk", "~> 2.6"
  spec.add_dependency "ed25519", "~> 1.3"
  spec.add_dependency "base58", "~> 0.2"
end
