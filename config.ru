# frozen_string_literal: true

require_relative "lib/recollect"

# Eagerly start sync workers (no-op when RECOLLECT_SYNC_DISABLE=1)
Recollect::HTTPServer.push_queue
Recollect::HTTPServer.sync_engine

run Recollect::HTTPServer
