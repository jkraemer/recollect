# frozen_string_literal: true

require "test_helper"

module Recollect
  class ConfigTest < TestCase
    def setup
      super
      @config = Config.new
    end

    def test_default_data_dir
      # When no env var set, should use ~/.recollect. HOME is pointed into the
      # test sandbox because Config.new creates the directories it points at -
      # otherwise this test writes the real ~/.recollect on whatever machine
      # runs it.
      fake_home = File.join(TEST_DATA_DIR, "fake_home")
      FileUtils.mkdir_p(fake_home)

      with_env("RECOLLECT_DATA_DIR" => nil, "HOME" => fake_home) do
        config = Config.new
        expected_path = File.join(fake_home, ".recollect")

        assert_equal Pathname.new(expected_path), config.data_dir
        assert_path_exists expected_path
      end
    end

    def test_data_dir_from_env
      # Should use RECOLLECT_DATA_DIR when set; the dir lives in the sandbox
      # because Config.new creates it.
      test_dir = File.join(TEST_DATA_DIR, "custom_data_dir")

      with_env("RECOLLECT_DATA_DIR" => test_dir) do
        config = Config.new

        assert_equal Pathname.new(test_dir), config.data_dir
      end
    end

    def test_default_host
      assert_equal "127.0.0.1", @config.host
    end

    def test_host_from_env
      with_env("RECOLLECT_HOST" => "0.0.0.0") do
        config = Config.new

        assert_equal "0.0.0.0", config.host
      end
    end

    def test_default_port
      assert_equal 7326, @config.port
    end

    def test_port_from_env
      with_env("RECOLLECT_PORT" => "9999") do
        config = Config.new

        assert_equal 9999, config.port
      end
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
      # @config was built in setup under whatever RECOLLECT_ENABLE_VECTORS
      # the process ambiently has, so force it off and build a fresh Config
      # here rather than asserting on @config's memoized value.
      with_env("RECOLLECT_ENABLE_VECTORS" => nil) do
        config = Config.new

        refute config.enable_vectors
      end
    end

    def test_vectors_enabled_from_env
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new

        assert config.enable_vectors
      end
    end

    def test_vector_dimensions
      assert_equal 384, @config.vector_dimensions
    end

    def test_embed_server_script_path
      expected = Recollect.root.join("bin", "embed-server")

      assert_equal expected, @config.embed_server_script_path
    end

    def test_vec_extension_path_finds_system_extension
      # Should find the sqlite-vec extension if installed. Force the env
      # override off so this exercises the system-path search regardless
      # of ambient RECOLLECT_SQLITE_VEC_PATH.
      with_env("RECOLLECT_SQLITE_VEC_PATH" => nil) do
        path = @config.vec_extension_path

        if File.exist?("/usr/lib/vec0.so")
          assert_equal "/usr/lib/vec0.so", path
        else
          # Extension not installed on this system - that's OK
          assert_nil(path) || assert_kind_of(String, path)
        end
      end
    end

    def test_vec_extension_path_prefers_env_override
      fake_vec = File.join(__dir__, "..", "tmp", "fake_vec0.so")
      File.write(fake_vec, "")

      with_env("RECOLLECT_SQLITE_VEC_PATH" => fake_vec) do
        assert_equal File.expand_path(fake_vec), Config.new.vec_extension_path
      end
    ensure
      FileUtils.rm_f(fake_vec)
    end

    def test_vec_extension_path_env_override_must_exist
      with_env("RECOLLECT_SQLITE_VEC_PATH" => "/nonexistent/vec0.so") do
        refute_equal "/nonexistent/vec0.so", Config.new.vec_extension_path
      end
    end

    def test_vectors_available_false_when_disabled
      # Force vectors off rather than relying on @config's memoized
      # enable_vectors, which reflects whatever the process ambiently has.
      with_env("RECOLLECT_ENABLE_VECTORS" => nil) do
        config = Config.new

        refute_predicate config, :vectors_available?
      end
    end

    def test_vectors_available_false_when_extension_missing
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new

        # Stub vec_extension_path to return nil
        def config.vec_extension_path
          nil
        end

        refute_predicate config, :vectors_available?
      end
    end

    def test_vectors_available_false_when_embed_script_missing
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new
        config.embed_server_script_path = Pathname.new("/nonexistent/embed-server")

        refute_predicate config, :vectors_available?
      end
    end

    def test_python_path_uses_venv_when_available
      # Should use .venv/bin/python3 if it exists and is executable. Force
      # the env override off so this exercises the venv/system fallback
      # regardless of ambient RECOLLECT_PYTHON.
      venv_python = Recollect.root.join(".venv", "bin", "python3")

      with_env("RECOLLECT_PYTHON" => nil) do
        if venv_python.executable?
          assert_equal venv_python.to_s, @config.python_path
        else
          assert_equal "python3", @config.python_path
        end
      end
    end

    # An installed gem has no writable .venv next to its code, so the
    # interpreter running the embedding model has to be nameable from outside.
    def test_python_path_prefers_env_override
      with_env("RECOLLECT_PYTHON" => "/opt/pythons/bin/python3") do
        assert_equal "/opt/pythons/bin/python3", Config.new.python_path
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
      # Force vectors off rather than relying on @config's memoized
      # enable_vectors, which reflects whatever the process ambiently has.
      with_env("RECOLLECT_ENABLE_VECTORS" => nil) do
        config = Config.new

        refute config.enable_vectors
        assert_equal "Vector embeddings: disabled (RECOLLECT_ENABLE_VECTORS not set)", config.vector_status_message
      end
    end

    def test_vector_status_message_when_extension_missing
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new

        def config.vec_extension_path
          nil
        end

        assert_equal "Vector embeddings: disabled (sqlite-vec extension not found)", config.vector_status_message
      end
    end

    def test_vector_status_message_when_embed_script_not_executable
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new
        config.embed_server_script_path = Pathname.new("/nonexistent/embed-server")

        # Need to also stub vec_extension_path to return a valid path
        def config.vec_extension_path
          "/some/path.so"
        end

        assert_equal "Vector embeddings: disabled (embed script not executable)", config.vector_status_message
      end
    end

    def test_vector_status_message_when_enabled
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
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
      end
    end

    # vector_unavailable_reason tests: the bare reason is the single home for
    # the cascade; vector_status_message and the status endpoint both wrap it.

    def test_vector_unavailable_reason_when_disabled_by_env
      with_env("RECOLLECT_ENABLE_VECTORS" => nil) do
        config = Config.new

        assert_equal "RECOLLECT_ENABLE_VECTORS not set", config.vector_unavailable_reason
      end
    end

    def test_vector_unavailable_reason_nil_when_available
      with_env("RECOLLECT_ENABLE_VECTORS" => "true") do
        config = Config.new

        def config.vec_extension_path
          "/some/path.so"
        end

        def config.embed_server_script_path
          Pathname.new("/bin/true")
        end

        assert_nil config.vector_unavailable_reason
      end
    end

    # max_vector_distance tests

    def test_default_max_vector_distance
      assert_in_delta 1.0, @config.max_vector_distance
    end

    def test_max_vector_distance_from_env
      with_env("RECOLLECT_MAX_VECTOR_DISTANCE" => "0.5") do
        config = Config.new

        assert_in_delta 0.5, config.max_vector_distance
      end
    end

    # Recency ranking tests

    def test_default_recency_aging_factor
      assert_in_delta 0.0, @config.recency_aging_factor
    end

    def test_default_recency_half_life_days
      assert_in_delta 30.0, @config.recency_half_life_days
    end

    def test_recency_disabled_by_default
      refute_predicate @config, :recency_enabled?
    end

    def test_recency_aging_factor_from_env
      with_env("RECOLLECT_RECENCY_AGING_FACTOR" => "0.7") do
        config = Config.new

        assert_in_delta 0.7, config.recency_aging_factor
      end
    end

    def test_recency_aging_factor_clamped_high
      with_env("RECOLLECT_RECENCY_AGING_FACTOR" => "1.5") do
        config = Config.new

        assert_in_delta 1.0, config.recency_aging_factor
      end
    end

    def test_recency_aging_factor_clamped_low
      with_env("RECOLLECT_RECENCY_AGING_FACTOR" => "-0.5") do
        config = Config.new

        assert_in_delta 0.0, config.recency_aging_factor
      end
    end

    def test_recency_half_life_days_from_env
      with_env("RECOLLECT_RECENCY_HALF_LIFE_DAYS" => "14") do
        config = Config.new

        assert_in_delta 14.0, config.recency_half_life_days
      end
    end

    def test_recency_enabled_when_aging_factor_positive
      with_env("RECOLLECT_RECENCY_AGING_FACTOR" => "0.5") do
        config = Config.new

        assert_predicate config, :recency_enabled?
      end
    end
  end
end
