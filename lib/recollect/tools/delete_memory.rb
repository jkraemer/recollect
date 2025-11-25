# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class DeleteMemory < MCP::Tool
      description 'Delete a specific memory by ID'

      input_schema(
        properties: {
          id: {
            type: 'integer',
            description: 'Memory ID to delete'
          },
          project: {
            type: 'string',
            description: 'Project name (omit for global)'
          }
        },
        required: ['id']
      )

      class << self
        def call(id:, project: nil, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          success = db.delete(id)

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              success: success,
              deleted_id: success ? id : nil,
              message: success ? 'Memory deleted' : 'Memory not found'
            })
          }])
        end
      end
    end
  end
end
