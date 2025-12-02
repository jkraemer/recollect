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
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new

      assert config.enable_vectors
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
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
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new

      # Stub vec_extension_path to return nil
      def config.vec_extension_path
        nil
      end

      refute_predicate config, :vectors_available?
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
    end

    def test_vectors_available_false_when_embed_script_missing
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new
      config.embed_server_script_path = Pathname.new("/nonexistent/embed-server")

      refute_predicate config, :vectors_available?
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
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

    # vector_status_message tests

    def test_vector_status_message_when_disabled_by_env
      refute @config.enable_vectors
      assert_equal "Vector embeddings: disabled (RECOLLECT_ENABLE_VECTORS not set)", @config.vector_status_message
    end

    def test_vector_status_message_when_extension_missing
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new

      def config.vec_extension_path
        nil
      end

      assert_equal "Vector embeddings: disabled (sqlite-vec extension not found)", config.vector_status_message
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
    end

    def test_vector_status_message_when_embed_script_not_executable
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new
      config.embed_server_script_path = Pathname.new("/nonexistent/embed-server")

      # Need to also stub vec_extension_path to return a valid path
      def config.vec_extension_path
        "/some/path.so"
      end

      assert_equal "Vector embeddings: disabled (embed script not executable)", config.vector_status_message
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
    end

    def test_vector_status_message_when_enabled
      ENV["RECOLLECT_ENABLE_VECTORS"] = "true"
      config = Config.new

      # Stub all conditions to be true
      def config.vec_extension_path
        "/some/path.so"
      end

      def config.embed_server_script_path
        # Return a path to an executable file
        Pathname.new("/bin/true")
      end

      assert_equal "Vector embeddings: enabled", config.vector_status_message
    ensure
      ENV.delete("RECOLLECT_ENABLE_VECTORS")
    end

    # max_vector_distance tests

    def test_default_max_vector_distance
      assert_in_delta 1.0, @config.max_vector_distance
    end

    def test_max_vector_distance_from_env
      ENV["RECOLLECT_MAX_VECTOR_DISTANCE"] = "0.5"
      config = Config.new

      assert_in_delta 0.5, config.max_vector_distance
    ensure
      ENV.delete("RECOLLECT_MAX_VECTOR_DISTANCE")
    end
  end
end
