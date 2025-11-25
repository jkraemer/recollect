# frozen_string_literal: true

require "rake/testtask"
require "rubocop/rake_task"

Rake::TestTask.new(:test) do |t|
  t.libs << "test"
  t.libs << "lib"
  t.test_files = FileList["test/**/*_test.rb"]
  t.warning = false
end

desc "Run tests with coverage report"
task :coverage do
  ENV["COVERAGE"] = "true"
  Rake::Task[:test].invoke
end

RuboCop::RakeTask.new

task default: :test
