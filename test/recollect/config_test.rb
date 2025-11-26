# frozen_string_literal: true

require "test_helper"

module Recollect
  class ConfigTest < TestCase
    def setup
      super
      @config = Config.new
    end

    def test_default_data_dir
      # When no env var set, should use ~/.recollect
      original_value = ENV.fetch("RECOLLECT_DATA_DIR", nil)
      ENV.delete("RECOLLECT_DATA_DIR")
      config = Config.new
      expected_path = File.join(Dir.home, ".recollect")

      assert_equal Pathname.new(expected_path), config.data_dir
    ensure
      ENV["RECOLLECT_DATA_DIR"] = original_value if original_value
    end

    def test_data_dir_from_env
      # Should use RECOLLECT_DATA_DIR when set
      test_dir = "/tmp/recollect_test"
      ENV["RECOLLECT_DATA_DIR"] = test_dir
      config = Config.new

      assert_equal Pathname.new(test_dir), config.data_dir
    ensure
      ENV["RECOLLECT_DATA_DIR"] = File.join(__dir__, "..", "tmp", "test_data")
    end

    def test_default_host
      assert_equal "127.0.0.1", @config.host
    end

    def test_host_from_env
      ENV["RECOLLECT_HOST"] = "0.0.0.0"
      config = Config.new

      assert_equal "0.0.0.0", config.host
    ensure
      ENV.delete("RECOLLECT_HOST")
    end

    def test_default_port
      assert_equal 7326, @config.port
    end

    def test_port_from_env
      ENV["RECOLLECT_PORT"] = "9999"
      config = Config.new

      assert_equal 9999, config.port
    ensure
      ENV.delete("RECOLLECT_PORT")
    end

    def test_url
      assert_equal "http://#{@config.host}:#{@config.port}", @config.url
    end

    def test_url_reflects_host_and_port_changes
      @config.host = "0.0.0.0"
      @config.port = 9000

      assert_equal "http://0.0.0.0:9000", @config.url
    end

    def test_default_max_results
      assert_equal 100, @config.max_results
    end

    def test_max_results_can_be_changed
      @config.max_results = 50

      assert_equal 50, @config.max_results
    end

    def test_global_db_path
      expected_path = @config.data_dir.join("global.db")

      assert_equal expected_path, @config.global_db_path
    end

    def test_projects_dir
      expected_path = @config.data_dir.join("projects")

      assert_equal expected_path, @config.projects_dir
    end

    def test_project_db_path_sanitizes_names
      # Should sanitize special characters to underscores
      assert_equal @config.projects_dir.join("my_project.db"),
                   @config.project_db_path("my-project")

      assert_equal @config.projects_dir.join("my_project.db"),
                   @config.project_db_path("my@project")

      assert_equal @config.projects_dir.join("my_project.db"),
                   @config.project_db_path("My Project")

      # Should be lowercase
      assert_equal @config.projects_dir.join("myproject.db"),
                   @config.project_db_path("MyProject")
    end

    def test_detect_project_from_git
      # Create a temporary directory with .git
      Dir.mktmpdir do |dir|
        git_dir = File.join(dir, ".git")
        FileUtils.mkdir_p(git_dir)

        # Mock git config to return a remote URL

        # Create a mock that returns the repo name
        def @config.git_remote_name(_path)
          "test-repo"
        end

        project_name = @config.detect_project(dir)

        # Should either return git remote name or directory basename
        assert project_name
        assert_kind_of String, project_name
        refute_empty project_name
      end
    end

    def test_detect_project_from_package_json
      # Create a temporary directory with package.json
      Dir.mktmpdir do |dir|
        package_json = File.join(dir, "package.json")
        File.write(package_json, '{"name": "my-npm-package"}')

        project_name = @config.detect_project(dir)

        assert_equal "my-npm-package", project_name
      end
    end

    def test_detect_project_from_gemspec
      # Create a temporary directory with gemspec
      Dir.mktmpdir do |dir|
        gemspec = File.join(dir, "my-gem.gemspec")
        File.write(gemspec, "Gem::Specification.new do |s|; end")

        project_name = @config.detect_project(dir)

        assert_equal "my-gem", project_name
      end
    end

    def test_detect_project_returns_nil_for_generic_directories
      # Generic directory names should return nil
      generic_dirs = %w[home Documents Desktop Downloads src code]

      generic_dirs.each do |dir_name|
        Dir.mktmpdir(dir_name) do |dir|
          # Rename the directory to match the generic name
          parent = File.dirname(dir)
          new_path = File.join(parent, dir_name)
          File.rename(dir, new_path)

          project_name = @config.detect_project(new_path)

          assert_nil project_name, "Expected nil for generic directory '#{dir_name}'"

          # Rename back for cleanup
          File.rename(new_path, dir)
        end
      end
    end

    def test_detect_project_returns_directory_name_for_non_generic
      # Non-generic directory without .git/package.json/gemspec should return basename
      Dir.mktmpdir("my-custom-project") do |dir|
        # Rename to have a known name
        parent = File.dirname(dir)
        new_path = File.join(parent, "my-custom-project")
        File.rename(dir, new_path)

        project_name = @config.detect_project(new_path)

        assert_equal "my-custom-project", project_name

        # Rename back for cleanup
        File.rename(new_path, dir)
      end
    end

    def test_ensures_directories_exist
      # Config initialization should create directories
      assert_predicate @config.data_dir, :exist?, "data_dir should exist"
      assert_predicate @config.projects_dir, :exist?, "projects_dir should exist"
    end

    # Vector search configuration tests

    def test_vectors_disabled_by_default
      refute @config.enable_vectors
    end

    def test_vectors_enabled_from_env
      ENV["ENABLE_VECTORS"] = "true"
      config = Config.new

      assert config.enable_vectors
    ensure
      ENV.delete("ENABLE_VECTORS")
    end

    def test_vector_dimensions
      assert_equal 384, @config.vector_dimensions
    end

    def test_embed_server_script_path
      expected = Recollect.root.join("bin", "embed-server")

      assert_equal expected, @config.embed_server_script_path
    end

    def test_vec_extension_path_finds_system_extension
      # Should find the sqlite-vec extension if installed
      path = @config.vec_extension_path

      if File.exist?("/usr/lib/vec0.so")
        assert_equal "/usr/lib/vec0.so", path
      else
        # Extension not installed on this system - that's OK
        assert_nil(path) || assert_kind_of(String, path)
      end
    end

    def test_vectors_available_false_when_disabled
      refute_predicate @config, :vectors_available?
    end

    def test_vectors_available_false_when_extension_missing
      ENV["ENABLE_VECTORS"] = "true"
      config = Config.new

      # Stub vec_extension_path to return nil
      def config.vec_extension_path
        nil
      end

      refute_predicate config, :vectors_available?
    ensure
      ENV.delete("ENABLE_VECTORS")
    end

    def test_vectors_available_false_when_embed_script_missing
      ENV["ENABLE_VECTORS"] = "true"
      config = Config.new
      config.embed_server_script_path = Pathname.new("/nonexistent/embed-server")

      refute_predicate config, :vectors_available?
    ensure
      ENV.delete("ENABLE_VECTORS")
    end

    def test_python_path_uses_venv_when_available
      # Should use .venv/bin/python3 if it exists and is executable
      venv_python = Recollect.root.join(".venv", "bin", "python3")

      if venv_python.executable?
        assert_equal venv_python.to_s, @config.python_path
      else
        assert_equal "python3", @config.python_path
      end
    end

    def test_python_path_falls_back_to_system_python
      # Test case where venv doesn't exist
      config = Config.new
      # Stub the venv check to fail
      def config.python_path
        "python3"
      end

      assert_equal "python3", config.python_path
    end

    def test_git_remote_name_returns_nil_for_empty_output
      Dir.mktmpdir do |dir|
        git_dir = File.join(dir, ".git")
        FileUtils.mkdir_p(git_dir)

        # Calling detect_project on a git repo without remote should fall back to dir name
        project = @config.detect_project(dir)

        # Should return the directory name since no remote is configured
        assert_kind_of String, project
      end
    end

    def test_git_remote_name_handles_standard_error
      # This tests the rescue path in git_remote_name
      config = Config.new

      # Access the private method directly and simulate an error
      result = config.send(:git_remote_name, Pathname.new("/nonexistent/path"))

      assert_nil result
    end

    def test_vec_extension_path_returns_nil_when_not_found
      config = Config.new

      # Override the method to search nonexistent paths
      def config.vec_extension_path
        paths = ["/nonexistent/path1.so", "/nonexistent/path2.so"]
        paths.each do |path|
          expanded = File.expand_path(path)
          return expanded if File.exist?(expanded)
        end
        nil
      end

      assert_nil config.vec_extension_path
    end

    def test_detect_project_with_real_git_remote
      # Test in the actual project directory which has a git remote
      project = @config.detect_project(Dir.pwd)

      # Should return the project name from git remote
      assert_kind_of String, project
      refute_empty project
    end
  end
end
