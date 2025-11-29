# frozen_string_literal: true

module Recollect
  # Encapsulates search filter parameters for memory queries.
  # Used by DatabaseManager and Database to reduce parameter list length.
  class SearchCriteria
    attr_reader :query, :project, :memory_type, :limit, :created_after, :created_before

    # rubocop:disable Metrics/ParameterLists -- This class exists to bundle these parameters
    def initialize(query:, project: nil, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      @query = query
      @project = project
      @memory_type = memory_type
      @limit = limit
      @created_after = created_after
      @created_before = created_before
    end
    # rubocop:enable Metrics/ParameterLists

    # Returns date filter options as a hash for passing to Database methods
    def date_opts
      { created_after:, created_before: }
    end

    # Returns true if a query is present (non-nil, non-empty)
    def query?
      return false if @query.nil?
      return false if @query.respond_to?(:empty?) && @query.empty?

      true
    end

    # Returns true if a project is specified
    def project?
      !@project.nil?
    end

    # Returns query as a string (joins array queries with space)
    def query_string
      @query.is_a?(Array) ? @query.join(" ") : @query
    end

    # Returns a new SearchCriteria with a different project
    def for_project(new_project)
      SearchCriteria.new(
        query:,
        project: new_project,
        memory_type:,
        limit:,
        created_after:,
        created_before:
      )
    end
  end
end
