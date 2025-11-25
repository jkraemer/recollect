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
      results = []

      if project
        # Search specific project only
        db = get_database(project)
        memories = db.search(query, memory_type: memory_type, limit: limit)
        memories.each { |m| m["project"] = project }
        results.concat(memories)
      else
        # Search global
        global_db = get_database(nil)
        global_memories = global_db.search(query, memory_type: memory_type, limit: limit)
        global_memories.each { |m| m["project"] = nil }
        results.concat(global_memories)

        # Search all projects
        list_projects.each do |proj|
          db = get_database(proj)
          proj_memories = db.search(query, memory_type: memory_type, limit: limit)
          proj_memories.each { |m| m["project"] = proj }
          results.concat(proj_memories)
        end
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

    def close_all
      @mutex.synchronize do
        @databases.each_value(&:close)
        @databases.clear
      end
    end

    private

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
  end
end
