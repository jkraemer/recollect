# frozen_string_literal: true

source "https://rubygems.org"

# Runtime dependencies live in recollect.gemspec
gemspec

group :development, :test do
  gem "minitest", "~> 5.25"
  gem "pry"
  gem "rack-test", "~> 2.1"
  gem "rake"
  gem "standard", "~> 1.44", require: false
  gem "rubocop-minitest", "~> 0.36", require: false
  gem "rubocop-rake", "~> 0.6", require: false
  gem "simplecov", "~> 0.22", require: false
  gem "simplecov-console", "~> 0.9", require: false
end

group :test do
  gem "faraday-rack", "~> 2.0"
end
