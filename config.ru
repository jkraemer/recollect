# frozen_string_literal: true

require_relative 'lib/recollect'
require 'rack/cors'

use Rack::Cors do
  allow do
    origins '*'
    resource '*', headers: :any, methods: %i[get post put delete options]
  end
end

run Recollect::HTTPServer
