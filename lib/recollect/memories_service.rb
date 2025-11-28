# frozen_string_literal: true

module Recollect
  class MemoriesService
    def initialize(db_manager)
      @db_manager = db_manager
    end

    def create(content:, project: nil, memory_type: nil, tags: [], source: "api")
      project = project&.downcase

      id = @db_manager.store_with_embedding(
        project: project,
        content: content,
        memory_type: memory_type || "note",
        tags: tags || [],
        metadata: nil,
        source: source
      )

      db = @db_manager.get_database(project)
      memory = db.get(id)
      memory["project"] = project
      memory
    end

    def get(id, project: nil)
      project = project&.downcase
      db = @db_manager.get_database(project)

      memory = db.get(id)
      return nil unless memory

      memory["project"] = project
      memory
    end

    def list(project: nil, memory_type: nil, limit: 50, offset: 0)
      project = project&.downcase
      db = @db_manager.get_database(project)

      memories = db.list(memory_type: memory_type, limit: limit, offset: offset)
      memories.each { |m| m["project"] = project }
      memories
    end

    def delete(id, project: nil)
      project = project&.downcase
      db = @db_manager.get_database(project)
      db.delete(id)
    end

    # rubocop:disable Metrics/ParameterLists
    def search(query, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      @db_manager.hybrid_search(
        query,
        project: project&.downcase,
        memory_type: memory_type,
        limit: limit,
        created_after: created_after,
        created_before: created_before
      )
    end

    def search_by_tags(tags, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      @db_manager.search_by_tags(
        tags,
        project: project&.downcase,
        memory_type: memory_type,
        limit: limit,
        created_after: created_after,
        created_before: created_before
      )
    end
    # rubocop:enable Metrics/ParameterLists

    def list_projects
      @db_manager.list_projects
    end

    def tag_stats(project: nil, memory_type: nil)
      @db_manager.tag_stats(
        project: project&.downcase,
        memory_type: memory_type
      )
    end
  end
end
