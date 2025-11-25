# frozen_string_literal: true

source "https://rubygems.org"

ruby ">= 3.4.0"

# Core
gem "mcp" # Official MCP SDK (Shopify)
gem "puma", "~> 6.4"
gem "rack-cors", "~> 2.0"
gem "sinatra", "~> 4.0"
gem "sqlite3", "~> 2.0"

# CLI
gem "pastel", "~> 0.8"
gem "thor", "~> 1.3"
gem "tty-table", "~> 0.12"

# Utilities
gem "zeitwerk", "~> 2.6" # Autoloading

group :development, :test do
  gem "minitest", "~> 5.25"
  gem "pry"
  gem "rack-test", "~> 2.1"
  gem "rake"
  gem "rubocop", "~> 1.69", require: false
  gem "rubocop-minitest", "~> 0.36", require: false
  gem "rubocop-rake", "~> 0.6", require: false
  gem "simplecov", "~> 0.22", require: false
  gem "simplecov-console", "~> 0.9", require: false
end
