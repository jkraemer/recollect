# frozen_string_literal: true

require "test_helper"
require "yaml"
require "open3"

# Recollect ships through two channels that have to stay in sync:
# the gem (server + CLI executables) and the Claude Code plugin
# (skill, command, MCP wiring). These tests guard both manifests.
class PackagingTest < Minitest::Test
  ROOT = File.expand_path("..", __dir__)

  def test_gemspec_is_valid_without_warnings
    warnings = capture_stderr { gemspec.validate }

    assert_empty warnings, "gemspec validation should be silent"
  end

  def test_gemspec_version_tracks_the_library
    assert_equal Recollect::VERSION, gemspec.version.to_s
  end

  def test_gemspec_installs_the_cli_and_the_server
    assert_equal %w[recollect recollect-server], gemspec.executables.sort

    gemspec.executables.each do |exe|
      path = File.join(ROOT, gemspec.bindir, exe)

      assert_path_exists path
      assert File.executable?(path), "#{path} must be executable"
    end
  end

  def test_gemspec_ships_the_files_the_server_needs_at_runtime
    %w[lib/recollect.rb config/puma.rb config.ru public/index.html bin/embed-server
      requirements.txt].each do |file|
      assert_includes gemspec.files, file
    end
  end

  def test_gemspec_omits_development_only_files
    refute(gemspec.files.any? { |f| f.start_with?("test/", "docs/") },
      "gem should not ship tests or internal docs")
    refute_includes gemspec.files, "bin/recollect", "bin/ wrappers are for the working copy only"
    refute_includes gemspec.files, "bin/server", "bin/ wrappers are for the working copy only"
  end

  def test_executables_do_not_depend_on_bundler
    gemspec.executables.each do |exe|
      source = File.read(File.join(ROOT, gemspec.bindir, exe))

      refute_match(/bundler\/setup/, source, "#{exe} must run from an installed gem, without bundler")
    end
  end

  # An installed gem is loaded by rubygems alone, so the library must require
  # everything it uses instead of relying on what bundler happens to pull in.
  def test_library_boots_outside_bundler
    out, status = Bundler.with_unbundled_env do
      Open3.capture2e(
        RbConfig.ruby, "-I#{File.join(ROOT, "lib")}", "-e",
        "require 'recollect'; Recollect.root.join('config', 'puma.rb'); Recollect.config"
      )
    end

    assert_predicate status, :success?, out
  end

  def test_plugin_manifest_describes_this_version
    assert_equal "recollect", plugin_manifest["name"]
    assert_equal Recollect::VERSION, plugin_manifest["version"]
    refute_empty plugin_manifest["description"].to_s
  end

  def test_plugin_ships_the_memory_skill
    skill = File.join(ROOT, "skills", "using-long-term-memory", "SKILL.md")

    assert_path_exists skill

    frontmatter = YAML.safe_load(File.read(skill).split("---")[1])

    assert_equal "using-long-term-memory", frontmatter["name"]
    refute_empty frontmatter["description"].to_s
  end

  def test_plugin_ships_the_session_log_command
    command = File.join(ROOT, "commands", "session-log.md")

    assert_path_exists command

    frontmatter = YAML.safe_load(File.read(command).split("---")[1])

    refute_empty frontmatter["description"].to_s, "the command needs a description to be discoverable"
  end

  def test_plugin_wires_the_mcp_endpoint
    server = mcp_config.fetch("mcpServers").fetch("recollect")

    assert_equal "http", server["type"]
    assert_equal "http://localhost:7326/mcp", server["url"]
  end

  def test_marketplace_lists_the_plugin_from_the_repository_root
    entry = marketplace_manifest.fetch("plugins").find { |p| p["name"] == "recollect" }

    refute_nil entry, "marketplace must list the recollect plugin"
    assert_equal "./", entry["source"]
    refute_empty marketplace_manifest["owner"]["name"].to_s
  end

  private

  def gemspec
    @gemspec ||= Gem::Specification.load(File.join(ROOT, "recollect.gemspec"))
  end

  def plugin_manifest
    @plugin_manifest ||= read_json(".claude-plugin/plugin.json")
  end

  def marketplace_manifest
    @marketplace_manifest ||= read_json(".claude-plugin/marketplace.json")
  end

  def mcp_config
    @mcp_config ||= read_json(".mcp.json")
  end

  def read_json(relative_path)
    JSON.parse(File.read(File.join(ROOT, relative_path)))
  end

  def capture_stderr
    original = $stderr
    $stderr = StringIO.new
    yield
    $stderr.string
  ensure
    $stderr = original
  end
end
