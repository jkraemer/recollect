# frozen_string_literal: true

require "json"

module Recollect
  class DatabaseManager
    def initialize(config = Recollect.config)
      @config = config
      @databases = {}
      @mutex = Mutex.new
    end

    def get_database(project = nil)
      key = project || :global

      @mutex.synchronize do
        @databases[key] ||= begin
          path = project ? @config.project_db_path(project) : @config.global_db_path

          # Store original project name for later retrieval
          store_project_metadata(project) if project

          Database.new(path)
        end
      end
    end

    def search_all(query, project: nil, memory_type: nil, limit: 10)
      results = if project
                  search_project(query, project, memory_type: memory_type, limit: limit)
                else
                  search_all_projects(query, memory_type: memory_type, limit: limit)
                end

      # Sort by relevance (rank) and limit
      results.sort_by { |m| m["rank"] || 0 }.take(limit)
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
      @mutex.synchronize do
        @databases.each_value(&:close)
        @databases.clear
      end
    end

    private

    def search_project(query, project, memory_type: nil, limit: 10)
      db = get_database(project)
      memories = db.search(query, memory_type: memory_type, limit: limit)
      memories.each { |m| m["project"] = project }
      memories
    end

    def search_all_projects(query, memory_type: nil, limit: 10)
      results = []

      # Search global
      results.concat(search_project(query, nil, memory_type: memory_type, limit: limit))

      # Search all projects
      list_projects.each do |proj|
        results.concat(search_project(query, proj, memory_type: memory_type, limit: limit))
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
  end
end
