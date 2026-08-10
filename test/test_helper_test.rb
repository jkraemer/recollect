# frozen_string_literal: true

require "test_helper"

module Recollect
  class WithEnvTest < TestCase
    def test_with_env_restores_previously_set_value
      ENV["RECOLLECT_TEST_VAR"] = "original"

      with_env("RECOLLECT_TEST_VAR" => "changed") do
        assert_equal "changed", ENV.fetch("RECOLLECT_TEST_VAR", nil)
      end

      assert_equal "original", ENV.fetch("RECOLLECT_TEST_VAR", nil)
    ensure
      ENV.delete("RECOLLECT_TEST_VAR")
    end

    def test_with_env_restores_absence
      ENV.delete("RECOLLECT_TEST_VAR")

      with_env("RECOLLECT_TEST_VAR" => "temporary") do
        assert_equal "temporary", ENV.fetch("RECOLLECT_TEST_VAR", nil)
      end

      refute ENV.key?("RECOLLECT_TEST_VAR")
    end

    def test_with_env_unsets_via_nil
      ENV["RECOLLECT_TEST_VAR"] = "original"

      with_env("RECOLLECT_TEST_VAR" => nil) do
        refute ENV.key?("RECOLLECT_TEST_VAR")
      end

      assert_equal "original", ENV.fetch("RECOLLECT_TEST_VAR", nil)
    ensure
      ENV.delete("RECOLLECT_TEST_VAR")
    end

    def test_with_env_restores_even_when_block_raises
      ENV["RECOLLECT_TEST_VAR"] = "original"

      assert_raises(RuntimeError) do
        with_env("RECOLLECT_TEST_VAR" => "changed") do
          raise "boom"
        end
      end

      assert_equal "original", ENV.fetch("RECOLLECT_TEST_VAR", nil)
    ensure
      ENV.delete("RECOLLECT_TEST_VAR")
    end

    def test_with_env_restores_multiple_keys
      ENV["RECOLLECT_TEST_VAR"] = "original"
      ENV.delete("RECOLLECT_TEST_VAR2")

      with_env("RECOLLECT_TEST_VAR" => "changed", "RECOLLECT_TEST_VAR2" => "new") do
        assert_equal "changed", ENV.fetch("RECOLLECT_TEST_VAR", nil)
        assert_equal "new", ENV.fetch("RECOLLECT_TEST_VAR2", nil)
      end

      assert_equal "original", ENV.fetch("RECOLLECT_TEST_VAR", nil)
      refute ENV.key?("RECOLLECT_TEST_VAR2")
    ensure
      ENV.delete("RECOLLECT_TEST_VAR")
      ENV.delete("RECOLLECT_TEST_VAR2")
    end
  end
end
