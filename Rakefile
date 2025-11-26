# frozen_string_literal: true

require "rake/testtask"
require "rubocop/rake_task"

# Fast tests (default) - excludes slow embedding/vector tests
Rake::TestTask.new(:test) do |t|
  t.libs << "test"
  t.libs << "lib"
  t.test_files = FileList["test/**/*_test.rb"].exclude(
    "test/recollect/embedding_*_test.rb",
    "test/recollect/database_vector_test.rb"
  )
  t.warning = false
end

# Slow embedding/vector tests only
Rake::TestTask.new("test:slow") do |t|
  t.libs << "test"
  t.libs << "lib"
  t.test_files = FileList[
    "test/recollect/embedding_*_test.rb",
    "test/recollect/database_vector_test.rb"
  ]
  t.warning = false
end

desc "Run all tests including slow embedding tests"
task "test:all" => [:test, "test:slow"]

desc "Run all tests with coverage report (in single process for accurate coverage)"
task :coverage do
  ENV["COVERAGE"] = "true"
  sh 'bundle exec ruby -Ilib -Itest -e "Dir[\'test/**/*_test.rb\'].each { |f| require File.expand_path(f) }"'
end

RuboCop::RakeTask.new

task default: :test
