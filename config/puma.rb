# frozen_string_literal: true

require_relative "../lib/recollect"

# Single process mode (no forking) - shares one EmbeddingClient/Python process
# Set WEB_CONCURRENCY > 0 to enable clustered mode if needed
workers ENV.fetch("WEB_CONCURRENCY", 0).to_i

# Threads per worker - handles concurrent requests
threads_count = ENV.fetch("PUMA_MAX_THREADS", 5).to_i
threads threads_count, threads_count

# Environment
environment ENV.fetch("RACK_ENV", "development")

# Binding - use Config for host/port defaults
bind "tcp://#{Recollect.config.host}:#{Recollect.config.port}"

# Preload app for copy-on-write memory savings
preload_app!

# Log startup configuration
puts "[Recollect] #{Recollect.config.vector_status_message}"

# Lifecycle hooks
before_worker_boot do
  # Each worker gets fresh DB connections via lazy initialization
end

# Allow puma to be restarted by `bin/puma --restart`
plugin :tmp_restart
