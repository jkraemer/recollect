# frozen_string_literal: true

require 'zeitwerk'

module Recollect
  class << self
    def config
      @config ||= Config.new
    end

    def configure
      yield config
    end

    def root
      Pathname.new(__dir__).parent
    end
  end
end

# Autoloading
loader = Zeitwerk::Loader.for_gem
loader.inflector.inflect('mcp_server' => 'MCPServer')
loader.inflector.inflect('http_server' => 'HTTPServer')
loader.setup
