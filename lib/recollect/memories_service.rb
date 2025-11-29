# frozen_string_literal: true

module Recollect
  class MemoriesService
    def initialize(db_manager)
      @db_manager = db_manager
    end

    def create(content:, project: nil, memory_type: nil, tags: [])
      project = project&.downcase

      id = @db_manager.store_with_embedding(
        project: project,
        content: content,
        memory_type: memory_type || "note",
        tags: tags || [],
        metadata: nil
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

    def search(criteria)
      criteria = normalize_project_in_criteria(criteria)
      @db_manager.hybrid_search(criteria)
    end

    def search_by_tags(criteria)
      criteria = normalize_project_in_criteria(criteria)
      @db_manager.search_by_tags(criteria)
    end

    def list_projects
      @db_manager.list_projects
    end

    def tag_stats(project: nil, memory_type: nil)
      @db_manager.tag_stats(
        project: project&.downcase,
        memory_type: memory_type
      )
    end

    private

    def normalize_project_in_criteria(criteria)
      return criteria unless criteria.project

      SearchCriteria.new(
        query: criteria.query,
        project: criteria.project.downcase,
        memory_type: criteria.memory_type,
        limit: criteria.limit,
        created_after: criteria.created_after,
        created_before: criteria.created_before
      )
    end
  end
end
