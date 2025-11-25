# frozen_string_literal: true

require 'mcp'

module Recollect
  module MCPServer
    TOOLS = [
      Tools::StoreMemory,
      Tools::SearchMemory,
      Tools::GetContext,
      Tools::ListProjects,
      Tools::DeleteMemory
    ].freeze

    class << self
      def build(db_manager)
        MCP::Server.new(
          name: 'recollect',
          version: Recollect::VERSION,
          tools: TOOLS,
          server_context: { db_manager: db_manager }
        )
      end
    end
  end
end
