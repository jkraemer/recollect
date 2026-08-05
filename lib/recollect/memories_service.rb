# frozen_string_literal: true

module Recollect
  class MemoriesService
    def initialize(db_manager, push_queue: nil)
      @db_manager = db_manager
      @push_queue = push_queue
    end

    def create(content:, project: nil, memory_type: nil, tags: [])
      project = project&.downcase
      if project == DatabaseManager::GLOBAL_DB_NAME
        # A project by this name would collide with the global database's
        # db_name in the sync mapping and could never sync.
        raise ArgumentError, "project name #{DatabaseManager::GLOBAL_DB_NAME.inspect} is reserved for the global database"
      end

      result = @db_manager.store_with_embedding(
        project: project,
        content: content,
        memory_type: memory_type || "note",
        tags: tags || [],
        metadata: nil
      )

      enqueue_push(result[:global_id], project)

      db = @db_manager.get_database(project)
      memory = db.get(result[:id])
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

    def list_all(memory_type: nil, limit: 50)
      results = []

      # Global memories
      results.concat(list(project: nil, memory_type: memory_type, limit: limit))

      # All project memories
      @db_manager.list_projects.each do |proj|
        results.concat(list(project: proj, memory_type: memory_type, limit: limit))
      end

      # Sort by created_at DESC and apply limit
      results.sort_by { |m| m["created_at"] || "" }.reverse.take(limit)
    end

    def delete(id, project: nil)
      project = project&.downcase
      db = @db_manager.get_database(project)
      # Capture global_id before tombstoning so we can enqueue a push for the deletion.
      row = db.instance_variable_get(:@db).get_first_row("SELECT global_id FROM memories WHERE id = ?", id)
      return false unless row

      success = db.delete(id)
      enqueue_push(row["global_id"], project) if success
      success
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

    def enqueue_push(global_id, project)
      return unless @push_queue

      @push_queue.enqueue(global_id: global_id, db_name: @db_manager.db_name_for_project(project))
    end

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
