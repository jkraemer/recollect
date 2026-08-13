# frozen_string_literal: true

require "sinatra/base"
require "json"
require "faraday"

module Recollect
  class HTTPServer < Sinatra::Base
    # Initialized eagerly so concurrent first calls share one mutex
    # instead of each thread making its own and locking nothing.
    @init_mutex = Mutex.new

    # Serializes the two-field resource refresh (the gem's resources= writes
    # @resources then @resource_index non-atomically) so concurrent requests
    # cannot leave list and index built from different snapshots.
    RESOURCE_REFRESH_MUTEX = Mutex.new

    configure do
      set :public_folder, proc { Recollect.root.join("public") }
      set :views, proc { Recollect.root.join("views") }
      set :show_exceptions, :after_handler
    end

    configure :development, :production do
      use Rack::CommonLogger, $stdout
    end

    # Class-level singletons to avoid file descriptor leaks.
    # Previously, helpers used per-request instance variables (@db_manager ||= ...)
    # which created new DatabaseManager instances (and SQLite connections) per request.
    #
    # Under Puma threading, bare ||= lazy init has a race window where two
    # concurrent first requests can each pass the nil check and initialize
    # separately. For local_identity, this would mint distinct Ed25519 keys
    # and INSERT separate local_identity rows. Subsequent reads (LIMIT 1 with
    # undefined order) could flip identities and silently invalidate pairings.
    # Double-checked locking keeps the hot path lock-free after first init.
    class << self
      attr_reader :init_mutex

      def sync_store
        return @sync_store if @sync_store

        init_mutex.synchronize { @sync_store ||= Sync::Store.new(Recollect.config.sync_db_path) }
      end

      def local_identity
        return @local_identity if @local_identity

        # Resolve sync_store outside the lock to avoid recursive acquisition.
        store = sync_store
        init_mutex.synchronize { @local_identity ||= Sync::Identity.ensure!(store) }
      end

      def db_manager
        return @db_manager if @db_manager

        # Resolve local_identity outside the lock to avoid recursive acquisition.
        identity = local_identity
        init_mutex.synchronize { @db_manager ||= DatabaseManager.new(Recollect.config, local_peer_id: identity.peer_id) }
      end

      def push_queue
        return nil if Recollect.config.sync_disabled?
        return @push_queue if @push_queue

        identity = local_identity
        manager = db_manager
        store = sync_store
        init_mutex.synchronize do
          @push_queue ||= begin
            pq = Sync::PushQueue.new(
              store: store,
              db_manager: manager,
              client_factory: ->(peer) {
                Sync::Client.new(peer_id: identity.peer_id, private_key: identity.private_key,
                  endpoint: peer[:endpoint])
              },
              size: Recollect.config.sync_push_queue_size
            )
            pq.start
            pq
          end
        end
      end

      def sync_engine
        return nil if Recollect.config.sync_disabled?
        return @sync_engine if @sync_engine

        identity = local_identity
        manager = db_manager
        store = sync_store
        init_mutex.synchronize do
          @sync_engine ||= begin
            engine = Sync::Engine.new(
              store: store,
              db_manager: manager,
              client_factory: ->(peer) {
                Sync::Client.new(peer_id: identity.peer_id, private_key: identity.private_key,
                  endpoint: peer[:endpoint])
              },
              heartbeat_seconds: Recollect.config.sync_heartbeat_seconds
            )
            engine.start
            engine
          end
        end
      end

      def memories_service
        @memories_service ||= MemoriesService.new(db_manager, push_queue: push_queue)
      end

      def mcp_server
        @mcp_server ||= MCPServer.build(db_manager, memories_service: memories_service)
      end

      def reset_db_manager!
        init_mutex.synchronize do
          @sync_engine&.stop
          @sync_engine = nil
          @push_queue&.stop
          @push_queue = nil
          @db_manager&.close_all
          @db_manager = nil
          @memories_service = nil
          @mcp_server = nil
          @sync_store&.close
          @sync_store = nil
          @local_identity = nil
        end
      end
    end

    # Mustermann anchors regex patterns (\A...\z), so this matches /sync/* but
    # NOT /api/sync/* (which has its own loopback gate via require_loopback!).
    before %r{/sync/.*} do
      body = ""
      if request.body
        request.body.rewind
        body = request.body.read
        request.body.rewind
      end
      result = Recollect::Sync::SignatureVerifier.new(self.class.sync_store).verify(headers: request.env, body: body)
      case result[:status]
      when :unauthorized then halt 401, {error: result[:reason]}.to_json
      when :forbidden then halt 403, {error: result[:reason]}.to_json
      else env["recollect.peer"] = result[:peer]
      end
    end

    # Wire dump logging for debugging
    before do
      if Recollect.config.log_wiredumps?
        @request_body = ""
        if request.body
          request.body.rewind
          @request_body = request.body.read
          request.body.rewind
        end

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
        self.class.db_manager
      end

      def memories_service
        self.class.memories_service
      end

      def mcp_server
        self.class.mcp_server
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
        halt 400, json_response({error: "Invalid JSON"}, status_code: 400)
      end

      def count_vectors_for_project(project)
        if project == "__all__"
          count_vectors_across_all_databases
        else
          db = db_manager.get_database(project)
          [db.count, db.embedding_count]
        end
      end

      def count_vectors_across_all_databases
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
        return "RECOLLECT_ENABLE_VECTORS not set" unless config.enable_vectors
        return "sqlite-vec extension not found" unless config.vec_extension_path
        return "embed script not executable" unless File.executable?(config.embed_server_script_path)

        "unknown"
      end

      def loopback_request?
        addr = request.env["REMOTE_ADDR"]
        %w[127.0.0.1 ::1].include?(addr)
      end

      def require_loopback!
        halt 403, {error: "local-only endpoint"}.to_json unless loopback_request?
      end

      def pairing_codes
        @pairing_codes ||= Recollect::Sync::PairingCodes.new(self.class.sync_store)
      end

      def peers_registry
        @peers_registry ||= Recollect::Sync::Peers.new(self.class.sync_store)
      end
    end

    # Health check
    get "/health" do
      json_response({status: "healthy", version: Recollect::VERSION})
    end

    # MCP endpoint (with and without trailing slash)
    post %r{/mcp/?} do
      request.body.rewind
      body = request.body.read
      content_type :json
      server = mcp_server
      # Rebuild the project resource list on every request (a Dir.glob over a
      # small directory) so a just-created project is listable and readable
      # immediately.
      RESOURCE_REFRESH_MUTEX.synchronize do
        server.resources = Recollect::Resources::Projects.build(
          db_manager: db_manager, memories_service: memories_service
        )
      end
      server.handle_json(body)
    end

    # ========== Pairing API (local-only) ==========

    post "/pairing/create" do
      require_loopback!

      result = pairing_codes.generate
      endpoint = Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
      content_type :json
      {code: result[:code], expires_at: result[:expires_at].iso8601, endpoint: endpoint}.to_json
    end

    post "/pairing/join" do
      body = JSON.parse(request.body.read)
      code = body["code"]
      joiner_id = body["peer_id"]

      halt 401, {error: "invalid or expired code"}.to_json unless pairing_codes.consume(code, used_by_peer: joiner_id)

      peers_registry.add(
        peer_id: joiner_id,
        display_name: body["display_name"],
        public_key: Base64.strict_decode64(body["public_key"]),
        endpoint: body["endpoint"],
        default_subscription: "global"
      )

      identity = self.class.local_identity
      content_type :json
      {
        peer_id: identity.peer_id,
        display_name: identity.display_name,
        public_key: Base64.strict_encode64(identity.public_key),
        endpoint: Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
      }.to_json
    end

    # ========== Sync API (local-only) ==========

    get "/api/sync/identity" do
      require_loopback!
      content_type :json
      identity = self.class.local_identity
      fingerprint = Digest::SHA256.hexdigest(identity.public_key)[0, 16]
      {peer_id: identity.peer_id, display_name: identity.display_name, public_key_fingerprint: fingerprint}.to_json
    end

    get "/api/sync/peers" do
      require_loopback!
      content_type :json
      peers_registry.list.map do |p|
        p.merge(subscriptions: peers_registry.subscriptions(p[:peer_id])).tap { |h| h.delete(:public_key) }
      end.to_json
    end

    delete "/api/sync/peers/:peer_id" do
      require_loopback!
      peers_registry.block(params[:peer_id])
      content_type :json
      {ok: true}.to_json
    end

    post "/api/sync/peers/:peer_id/subscriptions" do
      require_loopback!
      body = JSON.parse(request.body.read)
      peers_registry.subscribe(params[:peer_id], body["db_name"])
      content_type :json
      {ok: true}.to_json
    end

    delete "/api/sync/peers/:peer_id/subscriptions/:db_name" do
      require_loopback!
      peers_registry.unsubscribe(params[:peer_id], params[:db_name])
      content_type :json
      {ok: true}.to_json
    end

    post "/api/sync/sync" do
      require_loopback!
      engine = self.class.sync_engine
      halt 503, {error: "sync disabled"}.to_json unless engine

      body = request.body.read
      params_body = body.empty? ? {} : JSON.parse(body)

      peers_to_visit = if params_body["peer_id"]
        [peers_registry.find(params_body["peer_id"])].compact
      else
        peers_registry.list.select { |p| p[:status] == "trusted" }
      end

      results = peers_to_visit.flat_map do |peer|
        dbs = params_body["db_name"] ? [params_body["db_name"]] : peers_registry.subscriptions(peer[:peer_id])
        dbs.map do |db_name|
          engine.reconcile(peer: peer, db_name: db_name)
          {peer_id: peer[:peer_id], db_name: db_name, error: peers_registry.find(peer[:peer_id])[:last_sync_error]}
        end
      end

      content_type :json
      {results: results}.to_json
    end

    post "/api/sync/peers/join" do
      require_loopback!
      body = JSON.parse(request.body.read)
      code = body["code"]
      peer_endpoint = body["endpoint"]
      identity = self.class.local_identity

      payload = {
        code: code,
        peer_id: identity.peer_id,
        display_name: identity.display_name,
        public_key: Base64.strict_encode64(identity.public_key),
        endpoint: Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
      }

      response = Faraday.post("#{peer_endpoint}/pairing/join") do |req|
        req.headers["Content-Type"] = "application/json"
        req.body = payload.to_json
      end

      halt response.status, response.body unless response.success?

      remote = JSON.parse(response.body)
      peers_registry.add(
        peer_id: remote["peer_id"],
        display_name: remote["display_name"],
        public_key: Base64.strict_decode64(remote["public_key"]),
        endpoint: remote["endpoint"],
        default_subscription: "global"
      )

      content_type :json
      remote.merge("status" => "trusted").to_json
    end

    # ========== Sync API (signed, between peers) ==========

    get "/sync/manifest" do
      db_name = params["db"] || halt(400, {error: "missing db"}.to_json)
      project = self.class.db_manager.project_for_db_name(db_name)
      database = self.class.db_manager.get_database(project)
      identity = self.class.local_identity

      wm = Recollect::Sync::Watermarks.new(self.class.sync_store).get(db_name: db_name)
      self_cursor = database.max_origin_cursor(identity.peer_id)
      # Omit self entry when no local rows exist; receiver treats absence as
      # "send everything from this origin" via fetch_for_sync's empty-since branch.
      wm[identity.peer_id] = self_cursor if self_cursor

      content_type :json
      {watermarks: wm}.to_json
    end

    post "/sync/pull" do
      db_name = params["db"] || halt(400, {error: "missing db"}.to_json)
      caller_peer = env["recollect.peer"][:peer_id]
      peers = Recollect::Sync::Peers.new(self.class.sync_store)

      # Subscription is a sender-side policy: not subscribed means "I don't share this db with you."
      # Respond with an empty page rather than 403 so the caller can keep polling without re-pairing
      # if the policy changes later.
      unless peers.subscriptions(caller_peer).include?(db_name)
        content_type :json
        halt 200, {records: []}.to_json
      end

      payload = JSON.parse(request.body.read)
      since = payload["since"] || {}
      limit = (payload["limit"] || 500).to_i

      project = self.class.db_manager.project_for_db_name(db_name)
      rows = self.class.db_manager.get_database(project).fetch_for_sync(since: since, limit: limit)

      content_type :json
      # Local "id" is per-peer and not portable; strip it before sending.
      {records: rows.map { |r| r.except("id") }}.to_json
    end

    post "/sync/push" do
      db_name = params["db"] || halt(400, {error: "missing db"}.to_json)
      payload = JSON.parse(request.body.read)
      records = payload["records"] || []

      project = self.class.db_manager.project_for_db_name(db_name)
      database = self.class.db_manager.get_database(project)
      watermarks = Recollect::Sync::Watermarks.new(self.class.sync_store)

      accepted = 0
      # Track MAX (created_at, global_id) per origin observed in this batch.
      observed = Hash.new { |h, k| h[k] = {"created_at" => "", "global_id" => ""} }

      records.each do |rec|
        status = database.upsert_synced(rec)
        accepted += 1 if status != :no_change

        origin = rec["origin_peer"]
        cur = observed[origin]
        incoming = [rec["created_at"], rec["global_id"]]
        current = [cur["created_at"], cur["global_id"]]
        if (incoming <=> current) == 1
          observed[origin] = {"created_at" => rec["created_at"], "global_id" => rec["global_id"]}
        end

        # Re-embed only fresh INSERTs of non-tombstone, non-chunk records.
        next unless status == :inserted && rec["memory_type"] != "_chunk" && rec["deleted_at"].nil?

        row = database.instance_variable_get(:@db).get_first_row(
          "SELECT id FROM memories WHERE global_id = ?", rec["global_id"]
        )
        if row
          self.class.db_manager.enqueue_embedding(memory_id: row["id"], content: rec["content"], project: project)
        end
      end

      observed.each do |peer_id, cursor|
        next if cursor["created_at"].empty?

        watermarks.advance(peer_id: peer_id, db_name: db_name,
          created_at: cursor["created_at"], global_id: cursor["global_id"])
      end

      content_type :json
      {accepted: accepted, rejected: 0}.to_json
    end

    # ========== REST API ==========

    # List memories
    get "/api/memories" do
      project = params["project"]
      limit = (params["limit"] || 50).to_i
      memory_type = params["type"]

      memories = if project == "__all__"
        memories_service.list_all(memory_type: memory_type, limit: limit)
      else
        memories_service.list(
          project: project,
          memory_type: memory_type,
          limit: limit,
          offset: (params["offset"] || 0).to_i
        )
      end
      json_response(memories)
    end

    # Search memories (uses hybrid search when vectors available)
    get "/api/memories/search" do
      query = params["q"]
      halt 400, json_response({error: 'Query parameter "q" required'}, status_code: 400) unless query

      project = (params["project"] == "__all__") ? nil : params["project"]
      criteria = SearchCriteria.new(
        query: query,
        project: project,
        memory_type: params["type"],
        limit: (params["limit"] || 10).to_i
      )
      results = memories_service.search(criteria)

      json_response({results: results, count: results.length, query: query})
    end

    # Search by tags
    get "/api/memories/by-tags" do
      tags_param = params["tags"]
      halt 400, json_response({error: 'Query parameter "tags" required'}, status_code: 400) unless tags_param

      tags = tags_param.split(",").map(&:strip)
      project = (params["project"] == "__all__") ? nil : params["project"]

      criteria = SearchCriteria.new(
        query: tags,
        project: project,
        memory_type: params["memory_type"],
        limit: (params["limit"] || 10).to_i
      )
      results = memories_service.search_by_tags(criteria)

      json_response({results: results, count: results.length, tags: tags})
    end

    # Get single memory
    get "/api/memories/:id" do
      memory = memories_service.get(params["id"].to_i, project: params["project"])
      halt 404, json_response({error: "Memory not found"}, status_code: 404) unless memory

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
      halt 404, json_response({error: "Memory not found"}, status_code: 404) unless success

      json_response({deleted: params["id"].to_i})
    end

    # List projects
    get "/api/projects" do
      projects = memories_service.list_projects.reject { |p| p == "__all__" }
      json_response({projects: projects, count: projects.length})
    end

    # Tag statistics
    get "/api/tags" do
      tags = memories_service.tag_stats(
        project: params["project"],
        memory_type: params["memory_type"]
      )

      total = tags.values.sum
      unique = tags.size

      json_response({tags: tags, total: total, unique: unique})
    end

    # ========== Vector Search API ==========

    # Vector search status
    get "/api/vectors/status" do
      config = Recollect.config

      if config.vectors_available?
        project = params["project"]&.downcase
        project = nil if project&.empty?

        total_memories, total_embeddings = count_vectors_for_project(project)

        json_response({
          enabled: true,
          healthy: true,
          total_memories: total_memories,
          total_embeddings: total_embeddings,
          pending: total_memories - total_embeddings
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
        halt 400, json_response({error: "Vector search not enabled"}, status_code: 400)
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
      send_index
    end

    # Project-specific view (client-side routing)
    get "/projects/:project" do
      send_index
    end

    private

    def send_index
      index_path = File.join(settings.public_folder, "index.html")
      if File.exist?(index_path)
        send_file index_path
      else
        halt 404, json_response({error: "Web UI not installed"}, status_code: 404)
      end
    end

    # Error handling
    error ArgumentError do
      json_response({error: env["sinatra.error"].message}, status_code: 400)
    end

    error do
      json_response({error: env["sinatra.error"].message}, status_code: 500)
    end
  end
end
