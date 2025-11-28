# frozen_string_literal: true

require "sinatra/base"
require "json"

module Recollect
  class HTTPServer < Sinatra::Base
    configure do
      set :public_folder, proc { Recollect.root.join("public") }
      set :views, proc { Recollect.root.join("views") }
      set :show_exceptions, :after_handler
    end

    configure :development, :production do
      use Rack::CommonLogger, $stdout
    end

    # Wire dump logging for debugging
    before do
      if Recollect.config.log_wiredumps?
        request.body.rewind
        @request_body = request.body.read
        request.body.rewind

        $stdout.puts "[WIREDUMP] #{request.request_method} #{request.path_info}"
        $stdout.puts "[WIREDUMP] Headers: #{filtered_headers}"
        $stdout.puts "[WIREDUMP] Body: #{@request_body}" unless @request_body.empty?
      end
    end

    after do
      if Recollect.config.log_wiredumps?
        $stdout.puts "[WIREDUMP] Response status: #{response.status}"
        body_content = response.body.respond_to?(:join) ? response.body.join : response.body.to_s
        $stdout.puts "[WIREDUMP] Response: #{body_content}"
        $stdout.puts "[WIREDUMP] ---"
      end
    end

    helpers do
      def filtered_headers
        request.env.select { |k, _| k.start_with?("HTTP_") || k == "CONTENT_TYPE" }
      end

      def db_manager
        @db_manager ||= DatabaseManager.new(Recollect.config)
      end

      def memories_service
        @memories_service ||= MemoriesService.new(db_manager)
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
        halt 400, json_response({ error: "Invalid JSON" }, status_code: 400)
      end

      def count_vectors_across_databases
        total_memories = 0
        total_embeddings = 0

        db_manager.list_projects.each do |proj|
          db = db_manager.get_database(proj)
          total_memories += db.count
          total_embeddings += db.embedding_count
        end

        global_db = db_manager.get_database(nil)
        total_memories += global_db.count
        total_embeddings += global_db.embedding_count

        [total_memories, total_embeddings]
      end

      def determine_vector_unavailable_reason
        config = Recollect.config
        return "ENABLE_VECTORS not set" unless config.enable_vectors
        return "sqlite-vec extension not found" unless config.vec_extension_path
        return "embed script not executable" unless File.executable?(config.embed_server_script_path)

        "unknown"
      end
    end

    # Health check
    get "/health" do
      json_response({ status: "healthy", version: Recollect::VERSION })
    end

    # MCP endpoint (with and without trailing slash)
    post %r{/mcp/?} do
      request.body.rewind
      body = request.body.read
      content_type :json
      mcp_server.handle_json(body)
    end

    # ========== REST API ==========

    # List memories
    get "/api/memories" do
      memories = memories_service.list(
        project: params["project"],
        memory_type: params["type"],
        limit: (params["limit"] || 50).to_i,
        offset: (params["offset"] || 0).to_i
      )
      json_response(memories)
    end

    # Search memories (uses hybrid search when vectors available)
    get "/api/memories/search" do
      query = params["q"]
      halt 400, json_response({ error: 'Query parameter "q" required' }, status_code: 400) unless query

      results = memories_service.search(
        query,
        project: params["project"],
        memory_type: params["type"],
        limit: (params["limit"] || 10).to_i
      )

      json_response({ results: results, count: results.length, query: query })
    end

    # Search by tags
    get "/api/memories/by-tags" do
      tags_param = params["tags"]
      halt 400, json_response({ error: 'Query parameter "tags" required' }, status_code: 400) unless tags_param

      tags = tags_param.split(",").map(&:strip)

      results = memories_service.search_by_tags(
        tags,
        project: params["project"],
        memory_type: params["memory_type"],
        limit: (params["limit"] || 10).to_i
      )

      json_response({ results: results, count: results.length, tags: tags })
    end

    # Get single memory
    get "/api/memories/:id" do
      memory = memories_service.get(params["id"].to_i, project: params["project"])
      halt 404, json_response({ error: "Memory not found" }, status_code: 404) unless memory

      json_response(memory)
    end

    # Create memory (queues for embedding generation if vectors enabled)
    post "/api/memories" do
      data = parse_json_body

      memory = memories_service.create(
        content: data["content"],
        project: data["project"],
        memory_type: data["memory_type"],
        tags: data["tags"]
      )

      json_response(memory, status_code: 201)
    end

    # Delete memory
    delete "/api/memories/:id" do
      success = memories_service.delete(params["id"].to_i, project: params["project"])
      halt 404, json_response({ error: "Memory not found" }, status_code: 404) unless success

      json_response({ deleted: params["id"].to_i })
    end

    # List projects
    get "/api/projects" do
      projects = memories_service.list_projects
      json_response({ projects: projects, count: projects.length })
    end

    # Tag statistics
    get "/api/tags" do
      tags = memories_service.tag_stats(
        project: params["project"],
        memory_type: params["memory_type"]
      )

      total = tags.values.sum
      unique = tags.size

      json_response({ tags: tags, total: total, unique: unique })
    end

    # ========== Vector Search API ==========

    # Vector search status
    get "/api/vectors/status" do
      config = Recollect.config

      if config.vectors_available?
        total_memories, total_embeddings = count_vectors_across_databases

        json_response({
                        enabled: true,
                        healthy: true,
                        total_memories: total_memories,
                        total_embeddings: total_embeddings,
                        coverage: total_memories.positive? ? (total_embeddings.to_f / total_memories * 100).round(1) : 0
                      })
      else
        json_response({
                        enabled: false,
                        reason: determine_vector_unavailable_reason
                      })
      end
    end

    # Backfill embeddings for existing memories
    post "/api/vectors/backfill" do
      unless Recollect.config.vectors_available?
        halt 400, json_response({ error: "Vector search not enabled" }, status_code: 400)
      end

      project = params["project"]&.downcase
      limit = (params["limit"] || 100).to_i

      db = db_manager.get_database(project)
      pending = db.memories_without_embeddings(limit: limit)

      pending.each do |row|
        db_manager.enqueue_embedding(
          memory_id: row["id"],
          content: row["content"],
          project: project
        )
      end

      json_response({
                      success: true,
                      queued: pending.length,
                      message: "Queued #{pending.length} memories for embedding generation"
                    })
    end

    # Serve Web UI
    get "/" do
      index_path = File.join(settings.public_folder, "index.html")
      if File.exist?(index_path)
        send_file index_path
      else
        halt 404, json_response({ error: "Web UI not installed" }, status_code: 404)
      end
    end

    # Error handling
    error do
      json_response({ error: env["sinatra.error"].message }, status_code: 500)
    end
  end
end
