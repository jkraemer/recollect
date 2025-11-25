# frozen_string_literal: true

require_relative "../lib/recollect"

# Workers (processes)
workers ENV.fetch("WEB_CONCURRENCY", 2).to_i

# Threads per worker
threads_count = ENV.fetch("PUMA_MAX_THREADS", 5).to_i
threads threads_count, threads_count

# Environment
environment ENV.fetch("RACK_ENV", "development")

# Binding - use Config for host/port defaults
bind "tcp://#{Recollect.config.host}:#{Recollect.config.port}"

# Preload app for copy-on-write memory savings
preload_app!

# Lifecycle hooks
before_worker_boot do
  # Each worker gets fresh DB connections via lazy initialization
end

# Allow puma to be restarted by `bin/puma --restart`
plugin :tmp_restart
