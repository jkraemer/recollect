# frozen_string_literal: true

require 'sinatra/base'
require 'json'

module Recollect
  class HTTPServer < Sinatra::Base
    configure do
      set :public_folder, proc { Recollect.root.join('public') }
      set :views, proc { Recollect.root.join('views') }
      set :show_exceptions, :after_handler
    end

    helpers do
      def db_manager
        @db_manager ||= DatabaseManager.new(Recollect.config)
      end

      def mcp_server
        @mcp_server ||= MCPServer.build(db_manager)
      end

      def json_response(data, status_code: 200)
        content_type :json
        status status_code
        JSON.generate(data)
      end

      def parse_json_body
        request.body.rewind
        JSON.parse(request.body.read)
      rescue JSON::ParserError
        halt 400, json_response({ error: 'Invalid JSON' }, status_code: 400)
      end
    end

    # Health check
    get '/health' do
      json_response({ status: 'healthy', version: Recollect::VERSION })
    end

    # MCP endpoint
    post '/mcp' do
      request.body.rewind
      body = request.body.read
      content_type :json
      mcp_server.handle_json(body)
    end

    # ========== REST API ==========

    # List memories
    get '/api/memories' do
      project = params['project']
      memory_type = params['type']
      limit = (params['limit'] || 50).to_i
      offset = (params['offset'] || 0).to_i

      db = db_manager.get_database(project)
      memories = db.list(memory_type: memory_type, limit: limit, offset: offset)
      memories.each { |m| m['project'] = project }

      json_response(memories)
    end

    # Search memories
    get '/api/memories/search' do
      query = params['q']
      halt 400, json_response({ error: 'Query parameter "q" required' }, status_code: 400) unless query

      results = db_manager.search_all(
        query,
        project: params['project'],
        memory_type: params['type'],
        limit: (params['limit'] || 10).to_i
      )

      json_response({ results: results, count: results.length, query: query })
    end

    # Get single memory
    get '/api/memories/:id' do
      project = params['project']
      db = db_manager.get_database(project)

      memory = db.get(params['id'].to_i)
      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless memory

      memory['project'] = project
      json_response(memory)
    end

    # Create memory
    post '/api/memories' do
      data = parse_json_body
      project = data['project']
      db = db_manager.get_database(project)

      id = db.store(
        content: data['content'],
        memory_type: data['memory_type'] || 'note',
        tags: data['tags'],
        metadata: data['metadata'],
        source: 'api'
      )

      memory = db.get(id)
      memory['project'] = project

      json_response(memory, status_code: 201)
    end

    # Update memory
    put '/api/memories/:id' do
      data = parse_json_body
      project = data['project']
      db = db_manager.get_database(project)

      success = db.update(
        params['id'].to_i,
        content: data['content'],
        tags: data['tags'],
        metadata: data['metadata']
      )

      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless success

      memory = db.get(params['id'].to_i)
      memory['project'] = project
      json_response(memory)
    end

    # Delete memory
    delete '/api/memories/:id' do
      project = params['project']
      db = db_manager.get_database(project)

      success = db.delete(params['id'].to_i)
      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless success

      json_response({ deleted: params['id'].to_i })
    end

    # List projects
    get '/api/projects' do
      projects = db_manager.list_projects
      json_response({ projects: projects, count: projects.length })
    end

    # Serve Web UI
    get '/' do
      index_path = File.join(settings.public_folder, 'index.html')
      if File.exist?(index_path)
        send_file index_path
      else
        halt 404, json_response({ error: 'Web UI not installed' }, status_code: 404)
      end
    end

    # Error handling
    error do
      json_response({ error: env['sinatra.error'].message }, status_code: 500)
    end
  end
end
