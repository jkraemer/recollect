# frozen_string_literal: true

# Workers (processes)
workers ENV.fetch('WEB_CONCURRENCY', 2).to_i

# Threads per worker
threads_count = ENV.fetch('PUMA_MAX_THREADS', 5).to_i
threads threads_count, threads_count

# Environment
environment ENV.fetch('RACK_ENV', 'development')

# Binding
bind "tcp://#{ENV.fetch('RECOLLECT_HOST', '127.0.0.1')}:#{ENV.fetch('RECOLLECT_PORT', '8080')}"

# Preload app for copy-on-write memory savings
preload_app!

# Lifecycle hooks
on_worker_boot do
  # Each worker gets fresh DB connections via lazy initialization
end

# Allow puma to be restarted by `bin/puma --restart`
plugin :tmp_restart
