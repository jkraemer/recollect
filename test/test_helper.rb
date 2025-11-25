# frozen_string_literal: true

# SimpleCov must be started before any app code is loaded
if ENV["COVERAGE"]
  require "simplecov"
  require "simplecov-console"
  SimpleCov.start do
    add_filter "/test/"
    add_filter "/vendor/"
    enable_coverage :branch
    minimum_coverage line: 80, branch: 70
    formatter SimpleCov::Formatter::Console
  end
end

ENV["RACK_ENV"] = "test"

# Store test data directory in constant before any tests modify ENV
TEST_DATA_DIR = File.join(__dir__, "tmp", "test_data")
ENV["RECOLLECT_DATA_DIR"] = TEST_DATA_DIR

require "bundler/setup"
require "recollect"
require "minitest/autorun"
require "rack/test"
require "fileutils"
require "json"

# Ensure test data directory exists
FileUtils.mkdir_p(TEST_DATA_DIR)

module Recollect
  class TestCase < Minitest::Test
    include Rack::Test::Methods

    def setup
      # Clean databases between tests
      Dir.glob(File.join(TEST_DATA_DIR, "**/*.db*")).each do |f|
        FileUtils.rm_f(f)
      end
    end

    def teardown
      # Subclasses can override
    end
  end
end

Minitest.after_run do
  FileUtils.rm_rf(TEST_DATA_DIR)
end
