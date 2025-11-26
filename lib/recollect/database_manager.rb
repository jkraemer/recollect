# frozen_string_literal: true

require "json"

module Recollect
  class DatabaseManager
    def initialize(config = Recollect.config)
      @config = config
      @databases = {}
      @mutex = Mutex.new
      @embedding_worker = nil

      start_embedding_worker if @config.vectors_available?
    end

    def get_database(project = nil)
      project = project&.downcase
      key = project || :global

      @mutex.synchronize do
        @databases[key] ||= begin
          path = project ? @config.project_db_path(project) : @config.global_db_path

          # Store original project name for later retrieval
          store_project_metadata(project) if project

          Database.new(path, load_vectors: @config.vectors_available?)
        end
      end
    end

    def store_with_embedding(project:, content:, memory_type:, tags:, metadata:, source:)
      db = get_database(project)
      id = db.store(
        content: content,
        memory_type: memory_type,
        tags: tags,
        metadata: metadata,
        source: source
      )

      # Queue for embedding generation
      @embedding_worker&.enqueue(memory_id: id, content: content, project: project)

      id
    end

    def search_all(query, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      date_opts = { created_after:, created_before: }
      results = if project
                  search_project(query, project, memory_type: memory_type, limit: limit, **date_opts)
                else
                  search_all_projects(query, memory_type: memory_type, limit: limit, **date_opts)
                end

      # Sort by relevance (rank) and limit
      results.sort_by { |m| m["rank"] || 0 }.take(limit)
    end

    def search_by_tags(tags, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      date_opts = { created_after:, created_before: }
      results = if project
                  search_project_by_tags(tags, project, memory_type: memory_type, limit: limit, **date_opts)
                else
                  search_all_projects_by_tags(tags, memory_type: memory_type, limit: limit, **date_opts)
                end

      # Sort by created_at DESC and limit
      results.sort_by { |m| m["created_at"] || "" }.reverse.take(limit)
    end

    def hybrid_search(query, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      date_opts = { created_after:, created_before: }

      # If vectors not available, fall back to FTS5 only
      unless @config.vectors_available? && vectors_ready?
        return search_all(query, project: project, memory_type: memory_type, limit: limit, **date_opts)
      end

      # Get query embedding (convert array to space-joined string for embedding)
      embed_text = query.is_a?(Array) ? query.join(" ") : query
      embedding = embedding_client.embed(embed_text)

      # Collect results from both methods
      fts_results = search_all(query, project: project, memory_type: memory_type, limit: limit * 2, **date_opts)
      vec_results = vector_search_all(embedding, project: project, limit: limit * 2, **date_opts)

      # Merge and rank
      merge_hybrid_results(fts_results, vec_results, limit)
    end

    def list_projects
      @config.projects_dir.glob("*.db").map do |path|
        sanitized = path.basename(".db").to_s
        get_project_metadata(sanitized) || sanitized
      end.sort
    end

    def tag_stats(project: nil, memory_type: nil)
      if project
        get_database(project).get_tag_stats(memory_type: memory_type)
      else
        aggregate_tag_stats(memory_type: memory_type)
      end
    end

    def close_all
      @embedding_worker&.stop
      @embedding_client&.shutdown

      @mutex.synchronize do
        @databases.each_value(&:close)
        @databases.clear
      end
    end

    def enqueue_embedding(memory_id:, content:, project:)
      @embedding_worker&.enqueue(memory_id: memory_id, content: content, project: project)
    end

    private

    def search_project(query, project, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      project = project&.downcase
      db = get_database(project)
      memories = db.search(query, memory_type: memory_type, limit: limit,
                                  created_after:, created_before:)
      memories.each { |m| m["project"] = project }
      memories
    end

    def search_all_projects(query, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      results = []
      date_opts = { created_after:, created_before: }

      # Search global
      results.concat(search_project(query, nil, memory_type: memory_type, limit: limit, **date_opts))

      # Search all projects
      list_projects.each do |proj|
        results.concat(search_project(query, proj, memory_type: memory_type, limit: limit, **date_opts))
      end

      results
    end

    def search_project_by_tags(tags, project, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      project = project&.downcase
      db = get_database(project)
      memories = db.search_by_tags(tags, memory_type: memory_type, limit: limit,
                                         created_after:, created_before:)
      memories.each { |m| m["project"] = project }
      memories
    end

    def search_all_projects_by_tags(tags, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      results = []
      date_opts = { created_after:, created_before: }

      # Search global
      results.concat(search_project_by_tags(tags, nil, memory_type: memory_type, limit: limit, **date_opts))

      # Search all projects
      list_projects.each do |proj|
        results.concat(search_project_by_tags(tags, proj, memory_type: memory_type, limit: limit, **date_opts))
      end

      results
    end

    def store_project_metadata(project_name)
      metadata_file = @config.projects_dir.join(".project_names.json")
      metadata = load_project_metadata

      sanitized = sanitize_name(project_name)
      metadata[sanitized] = project_name

      File.write(metadata_file, JSON.generate(metadata))
    end

    def get_project_metadata(sanitized_name)
      metadata = load_project_metadata
      metadata[sanitized_name]
    end

    def load_project_metadata
      metadata_file = @config.projects_dir.join(".project_names.json")
      return {} unless metadata_file.exist?

      JSON.parse(metadata_file.read)
    rescue JSON::ParserError
      {}
    end

    def sanitize_name(name)
      name.to_s.gsub(/[^a-zA-Z0-9_]/, "_").downcase
    end

    def aggregate_tag_stats(memory_type: nil)
      combined = Hash.new(0)

      # Global database
      get_database(nil).get_tag_stats(memory_type: memory_type).each do |tag, count|
        combined[tag] += count
      end

      # All project databases
      list_projects.each do |proj|
        get_database(proj).get_tag_stats(memory_type: memory_type).each do |tag, count|
          combined[tag] += count
        end
      end

      # Sort by frequency descending
      combined.sort_by { |_, count| -count }.to_h
    end

    # Vector search helpers

    def start_embedding_worker
      @embedding_worker = EmbeddingWorker.new(self)
      @embedding_worker.start
    end

    def embedding_client
      @embedding_client ||= EmbeddingClient.new
    end

    def vectors_ready?
      # Check if at least one database has vectors enabled
      @databases.values.any?(&:vectors_enabled?)
    end

    def vector_search_all(embedding, project: nil, limit: 10, created_after: nil, created_before: nil)
      project = project&.downcase
      date_opts = { created_after:, created_before: }

      if project
        db = get_database(project)
        results = db.vector_search(embedding, limit: limit, **date_opts)
        results.each { |m| m["project"] = project }
      else
        results = []

        # Search global
        global_results = get_database(nil).vector_search(embedding, limit: limit, **date_opts)
        global_results.each { |m| m["project"] = nil }
        results.concat(global_results)

        # Search all projects
        list_projects.each do |proj|
          proj_results = get_database(proj).vector_search(embedding, limit: limit, **date_opts)
          proj_results.each { |m| m["project"] = proj }
          results.concat(proj_results)
        end

      end
      results
    end

    def merge_hybrid_results(fts_results, vec_results, limit)
      scores = {}
      score_fts_results(fts_results, scores)
      score_vector_results(vec_results, scores)
      combine_and_sort_scores(scores, limit)
    end

    def score_fts_results(fts_results, scores)
      max_rank = fts_results.map { |m| (m["rank"] || 0).abs }.max
      max_rank = 1.0 if max_rank.nil? || max_rank.zero?
      fts_results.each do |mem|
        normalized = (mem["rank"] || 0).abs / max_rank
        scores[mem["id"]] = { memory: mem, fts_score: normalized, vec_score: 0.0 }
      end
    end

    def score_vector_results(vec_results, scores)
      max_distance = vec_results.map { |m| m["distance"] || 0 }.max
      max_distance = 1.0 if max_distance.nil? || max_distance.zero?
      vec_results.each do |mem|
        normalized = 1.0 - ((mem["distance"] || 0) / max_distance)
        if scores[mem["id"]]
          scores[mem["id"]][:vec_score] = normalized
        else
          scores[mem["id"]] = { memory: mem, fts_score: 0.0, vec_score: normalized }
        end
      end
    end

    def combine_and_sort_scores(scores, limit)
      scored = scores.values.map do |entry|
        combined = (entry[:fts_score] * 0.6) + (entry[:vec_score] * 0.4)
        entry[:memory].merge("combined_score" => combined)
      end
      scored.sort_by { |m| -m["combined_score"] }.take(limit)
    end
  end
end
