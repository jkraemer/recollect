# frozen_string_literal: true

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
      project = sanitize_project_name(project) if project
      key = project || :global

      @mutex.synchronize do
        @databases[key] ||= begin
          path = project ? project_db_path(project) : @config.global_db_path
          Database.new(path, load_vectors: @config.vectors_available?)
        end
      end
    end

    def store_with_embedding(project:, content:, memory_type:, tags:, metadata:)
      db = get_database(project)
      id = db.store(
        content: content,
        memory_type: memory_type,
        tags: tags,
        metadata: metadata
      )

      # Queue for embedding generation
      @embedding_worker&.enqueue(memory_id: id, content: content, project: project)

      id
    end

    def search_all(criteria)
      results = if criteria.project?
                  search_project(criteria)
                else
                  search_all_projects(criteria)
                end

      # Sort by relevance (rank) and limit
      results.sort_by { |m| m["rank"] || 0 }.take(criteria.limit)
    end

    def search_by_tags(criteria)
      results = if criteria.project?
                  search_project_by_tags(criteria)
                else
                  search_all_projects_by_tags(criteria)
                end

      # Sort by created_at DESC and limit
      results.sort_by { |m| m["created_at"] || "" }.reverse.take(criteria.limit)
    end

    def hybrid_search(criteria)
      # If vectors not available, fall back to FTS5 only
      return search_all(criteria) unless @config.vectors_available? && vectors_ready?

      # Get query embedding
      embed_text = criteria.query_string
      embedding = embedding_client.embed(embed_text)

      # Collect results from both methods using doubled limit
      expanded_criteria = SearchCriteria.new(
        query: criteria.query,
        project: criteria.project,
        memory_type: criteria.memory_type,
        limit: criteria.limit * 2,
        created_after: criteria.created_after,
        created_before: criteria.created_before
      )
      fts_results = search_all(expanded_criteria)
      vec_results = vector_search_all(embedding, expanded_criteria)

      # Merge and rank
      merge_hybrid_results(fts_results, vec_results, criteria.limit)
    end

    def list_projects
      @config.projects_dir.glob("*.db").map do |path|
        path.basename(".db").to_s
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

    def search_project(criteria)
      project = criteria.project&.downcase
      db = get_database(project)
      memories = db.search(criteria.query,
                           memory_type: criteria.memory_type,
                           limit: criteria.limit,
                           **criteria.date_opts)
      memories.each { |m| m["project"] = project }
      memories
    end

    def search_all_projects(criteria)
      results = []

      # Search global
      results.concat(search_project(criteria.for_project(nil)))

      # Search all projects
      list_projects.each do |proj|
        results.concat(search_project(criteria.for_project(proj)))
      end

      results
    end

    def search_project_by_tags(criteria)
      project = criteria.project&.downcase
      db = get_database(project)
      memories = db.search_by_tags(criteria.query,
                                   memory_type: criteria.memory_type,
                                   limit: criteria.limit,
                                   **criteria.date_opts)
      memories.each { |m| m["project"] = project }
      memories
    end

    def search_all_projects_by_tags(criteria)
      results = []

      # Search global
      results.concat(search_project_by_tags(criteria.for_project(nil)))

      # Search all projects
      list_projects.each do |proj|
        results.concat(search_project_by_tags(criteria.for_project(proj)))
      end

      results
    end

    def sanitize_project_name(name)
      name.to_s.gsub(/[^a-zA-Z0-9_]/, "_").downcase
    end

    def project_db_path(project_name)
      @config.projects_dir.join("#{project_name}.db")
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

    def vector_search_all(embedding, criteria)
      project = criteria.project&.downcase

      if project
        db = get_database(project)
        results = db.vector_search(embedding, limit: criteria.limit, **criteria.date_opts)
        results.each { |m| m["project"] = project }
      else
        results = []

        # Search global
        global_results = get_database(nil).vector_search(embedding, limit: criteria.limit, **criteria.date_opts)
        global_results.each { |m| m["project"] = nil }
        results.concat(global_results)

        # Search all projects
        list_projects.each do |proj|
          proj_results = get_database(proj).vector_search(embedding, limit: criteria.limit, **criteria.date_opts)
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
