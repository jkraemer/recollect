# Recollect - Ruby MCP Memory Server

A Ruby-based HTTP MCP server for persistent memory management across Claude Code sessions.

---

## Overview

**Recollect** provides persistent memory for AI coding assistants. It stores decisions, patterns, bugs, and learnings in SQLite databases with full-text search, accessible via MCP protocol over HTTP.

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   Sinatra/Puma Server                   │
├─────────────────────────────────────────────────────────┤
│  POST /mcp         → MCP::Server#handle_json(body)      │
│  GET/POST /api/*   → REST endpoints for Web UI + CLI    │
│  GET /             → Static Web UI files                │
└─────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────┐
│              SQLite + FTS5 (per-project)                │
├─────────────────────────────────────────────────────────┤
│  ~/.recollect/global.db        → Cross-project memories │
│  ~/.recollect/projects/*.db    → Project-specific       │
└─────────────────────────────────────────────────────────┘
```

### Key Design Decisions

- **HTTP only** - Single Puma server as central access point; simplifies SQLite concurrency
- **MCP via `handle_json`** - No stdio transport; MCP protocol exposed as HTTP endpoint
- **Embedding-ready** - Schema includes BLOB column for future vector search
- **Project isolation** - Separate database per project, plus global database

---

## Project Structure

```
recollect/
├── Gemfile
├── Gemfile.lock
├── Rakefile                     # Test tasks
├── config.ru                    # Rack config
├── config/
│   └── puma.rb                  # Puma settings
├── lib/
│   ├── recollect.rb             # Main module, configuration
│   └── recollect/
│       ├── version.rb
│       ├── config.rb            # Configuration class
│       ├── database.rb          # SQLite wrapper
│       ├── database_manager.rb  # Multi-DB coordination
│       ├── mcp_server.rb        # MCP::Server factory
│       ├── http_server.rb       # Sinatra application
│       └── tools/               # MCP tool classes
│           ├── store_memory.rb
│           ├── search_memory.rb
│           ├── get_context.rb
│           ├── list_projects.rb
│           └── delete_memory.rb
├── public/                      # Web UI static files
│   ├── index.html
│   ├── style.css
│   └── app.js
├── test/
│   ├── test_helper.rb
│   ├── recollect/
│   │   ├── database_test.rb
│   │   ├── database_manager_test.rb
│   │   └── http_server_test.rb
│   └── integration/
│       └── mcp_test.rb
└── bin/
    ├── server                   # Start Puma
    └── recollect                # CLI tool
```

---

## Dependencies

### Gemfile

```ruby
# frozen_string_literal: true

source 'https://rubygems.org'

ruby '>= 3.4.0'

# Core
gem 'mcp'                        # Official MCP SDK (Shopify)
gem 'sinatra', '~> 4.0'
gem 'puma', '~> 6.4'
gem 'sqlite3', '~> 2.0'
gem 'rack-cors', '~> 2.0'

# CLI
gem 'thor', '~> 1.3'
gem 'tty-table', '~> 0.12'
gem 'pastel', '~> 0.8'

# Utilities
gem 'zeitwerk', '~> 2.6'         # Autoloading

group :development, :test do
  gem 'minitest', '~> 5.25'
  gem 'rack-test', '~> 2.1'
  gem 'pry'
end
```

---

## Core Implementation

### 1. Main Module (`lib/recollect.rb`)

```ruby
# frozen_string_literal: true

require 'zeitwerk'

module Recollect
  class << self
    def config
      @config ||= Config.new
    end

    def configure
      yield config
    end

    def root
      Pathname.new(__dir__).parent
    end
  end
end

# Autoloading
loader = Zeitwerk::Loader.for_gem
loader.setup
```

### 2. Configuration (`lib/recollect/config.rb`)

```ruby
# frozen_string_literal: true

require 'pathname'

module Recollect
  class Config
    attr_accessor :data_dir, :host, :port, :max_results

    def initialize
      @data_dir = Pathname.new(ENV.fetch('RECOLLECT_DATA_DIR',
        File.join(Dir.home, '.recollect')))
      @host = ENV.fetch('RECOLLECT_HOST', '127.0.0.1')
      @port = ENV.fetch('RECOLLECT_PORT', '8080').to_i
      @max_results = 100

      ensure_directories!
    end

    def global_db_path
      data_dir.join('global.db')
    end

    def projects_dir
      data_dir.join('projects')
    end

    def project_db_path(project_name)
      projects_dir.join("#{sanitize_name(project_name)}.db")
    end

    def detect_project(cwd = Dir.pwd)
      path = Pathname.new(cwd)

      # Check for .git
      if (path / '.git').exist?
        return git_remote_name(path) || path.basename.to_s
      end

      # Check for package.json
      if (path / 'package.json').exist?
        data = JSON.parse((path / 'package.json').read)
        return data['name'] if data['name']
      end

      # Check for *.gemspec
      gemspec = Dir.glob(path / '*.gemspec').first
      return File.basename(gemspec, '.gemspec') if gemspec

      # Fallback to directory name (unless generic)
      name = path.basename.to_s
      return nil if %w[home Documents Desktop Downloads src code].include?(name)

      name
    end

    private

    def ensure_directories!
      data_dir.mkpath
      projects_dir.mkpath
    end

    def sanitize_name(name)
      name.to_s.gsub(/[^a-zA-Z0-9_-]/, '_').downcase
    end

    def git_remote_name(path)
      output = `git -C "#{path}" config --get remote.origin.url 2>/dev/null`.strip
      return nil if output.empty?

      # Extract repo name from URL
      output.split('/').last&.sub(/\.git$/, '')
    rescue StandardError
      nil
    end
  end
end
```

### 3. Database (`lib/recollect/database.rb`)

```ruby
# frozen_string_literal: true

require 'sqlite3'
require 'json'

module Recollect
  class Database
    SCHEMA = <<~SQL
      -- Main memories table
      CREATE TABLE IF NOT EXISTS memories (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        content TEXT NOT NULL,
        memory_type TEXT NOT NULL DEFAULT 'note',
        tags TEXT,
        metadata TEXT,
        embedding BLOB,
        created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        source TEXT DEFAULT 'unknown'
      );

      -- FTS5 virtual table for full-text search
      CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
        content,
        tags,
        memory_type,
        content='memories',
        content_rowid='id'
      );

      -- Indexes
      CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
      CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);

      -- Triggers to keep FTS in sync
      CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
        INSERT INTO memories_fts(rowid, content, tags, memory_type)
        VALUES (new.id, new.content, new.tags, new.memory_type);
      END;

      CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
        DELETE FROM memories_fts WHERE rowid = old.id;
      END;

      CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
        DELETE FROM memories_fts WHERE rowid = old.id;
        INSERT INTO memories_fts(rowid, content, tags, memory_type)
        VALUES (new.id, new.content, new.tags, new.memory_type);
      END;
    SQL

    def initialize(db_path)
      @db_path = db_path.to_s
      @db = SQLite3::Database.new(@db_path)
      @db.results_as_hash = true
      configure_database
      create_schema
    end

    def store(content:, memory_type: 'note', tags: nil, metadata: nil, source: 'unknown')
      @db.execute(<<~SQL, [content, memory_type, json_encode(tags), json_encode(metadata), source])
        INSERT INTO memories (content, memory_type, tags, metadata, source)
        VALUES (?, ?, ?, ?, ?)
      SQL
      @db.last_insert_row_id
    end

    def get(id)
      row = @db.get_first_row('SELECT * FROM memories WHERE id = ?', id)
      deserialize(row)
    end

    def search(query, memory_type: nil, limit: 10, offset: 0)
      # Escape query for FTS5 (treat as literal phrase)
      safe_query = '"' + query.gsub('"', '""') + '"'

      sql = <<~SQL
        SELECT memories.*, bm25(memories_fts) as rank
        FROM memories_fts
        JOIN memories ON memories.id = memories_fts.rowid
        WHERE memories_fts MATCH ?
      SQL
      params = [safe_query]

      if memory_type
        sql += ' AND memories.memory_type = ?'
        params << memory_type
      end

      sql += ' ORDER BY rank LIMIT ? OFFSET ?'
      params.concat([limit, offset])

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def list(memory_type: nil, limit: 50, offset: 0)
      sql = 'SELECT * FROM memories'
      params = []

      if memory_type
        sql += ' WHERE memory_type = ?'
        params << memory_type
      end

      sql += ' ORDER BY created_at DESC LIMIT ? OFFSET ?'
      params.concat([limit, offset])

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def update(id, content: nil, tags: nil, metadata: nil)
      updates = []
      params = []

      if content
        updates << 'content = ?'
        params << content
      end

      if tags
        updates << 'tags = ?'
        params << json_encode(tags)
      end

      if metadata
        updates << 'metadata = ?'
        params << json_encode(metadata)
      end

      return false if updates.empty?

      updates << "updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"
      params << id

      @db.execute("UPDATE memories SET #{updates.join(', ')} WHERE id = ?", params)
      @db.changes > 0
    end

    def delete(id)
      @db.execute('DELETE FROM memories WHERE id = ?', id)
      @db.changes > 0
    end

    def count(memory_type: nil)
      if memory_type
        @db.get_first_value('SELECT COUNT(*) FROM memories WHERE memory_type = ?', memory_type)
      else
        @db.get_first_value('SELECT COUNT(*) FROM memories')
      end
    end

    def close
      @db.close
    end

    private

    def configure_database
      @db.execute_batch(<<~SQL)
        PRAGMA journal_mode = WAL;
        PRAGMA busy_timeout = 5000;
        PRAGMA synchronous = NORMAL;
        PRAGMA cache_size = 10000;
        PRAGMA temp_store = MEMORY;
      SQL
    end

    def create_schema
      @db.execute_batch(SCHEMA)
    end

    def json_encode(value)
      value ? JSON.generate(value) : nil
    end

    def deserialize(row)
      return nil unless row

      {
        'id' => row['id'],
        'content' => row['content'],
        'memory_type' => row['memory_type'],
        'tags' => json_decode(row['tags']),
        'metadata' => json_decode(row['metadata']),
        'created_at' => row['created_at'],
        'updated_at' => row['updated_at'],
        'source' => row['source'],
        'rank' => row['rank']
      }.compact
    end

    def json_decode(value)
      return nil unless value

      JSON.parse(value)
    rescue JSON::ParserError
      nil
    end
  end
end
```

### 4. Database Manager (`lib/recollect/database_manager.rb`)

```ruby
# frozen_string_literal: true

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
        memories.each { |m| m['project'] = project }
        results.concat(memories)
      else
        # Search global
        global_db = get_database(nil)
        global_memories = global_db.search(query, memory_type: memory_type, limit: limit)
        global_memories.each { |m| m['project'] = nil }
        results.concat(global_memories)

        # Search all projects
        list_projects.each do |proj|
          db = get_database(proj)
          proj_memories = db.search(query, memory_type: memory_type, limit: limit)
          proj_memories.each { |m| m['project'] = proj }
          results.concat(proj_memories)
        end
      end

      # Sort by relevance (rank) and limit
      results.sort_by { |m| m['rank'] || 0 }.take(limit)
    end

    def list_projects
      @config.projects_dir.glob('*.db').map do |path|
        path.basename('.db').to_s
      end.sort
    end

    def close_all
      @mutex.synchronize do
        @databases.each_value(&:close)
        @databases.clear
      end
    end
  end
end
```

---

## MCP Tools

### 5. Store Memory Tool (`lib/recollect/tools/store_memory.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class StoreMemory < MCP::Tool
      description <<~DESC
        Store a memory for later retrieval.

        AUTOMATIC TRIGGERING - Use this tool when you observe:

        Decisions & Architecture:
        - User makes architectural decisions
        - User explains why they chose an approach
        - User discusses trade-offs between options

        Bug Solutions:
        - User describes a bug and its solution
        - User explains workarounds
        - User documents known issues

        Patterns & Conventions:
        - User establishes coding patterns
        - User defines project conventions

        Trigger Phrases (store immediately):
        - "remember that..."
        - "for future reference..."
        - "we decided..."
        - "the solution is..."

        IMPORTANT: Store proactively! Don't wait for explicit commands.
      DESC

      input_schema(
        properties: {
          content: {
            type: 'string',
            description: 'The memory content to store'
          },
          memory_type: {
            type: 'string',
            enum: %w[note decision pattern bug learning],
            description: 'Type of memory (default: note)'
          },
          tags: {
            type: 'array',
            items: { type: 'string' },
            description: 'Tags for categorization'
          },
          project: {
            type: 'string',
            description: 'Project name (omit for global memory)'
          }
        },
        required: ['content']
      )

      class << self
        def call(content:, memory_type: 'note', tags: nil, project: nil, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          id = db.store(
            content: content,
            memory_type: memory_type,
            tags: tags,
            source: 'mcp'
          )

          location = project ? "project '#{project}'" : 'global'

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              success: true,
              id: id,
              stored_in: location,
              message: "Memory stored successfully in #{location}"
            })
          }])
        end
      end
    end
  end
end
```

### 6. Search Memory Tool (`lib/recollect/tools/search_memory.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class SearchMemory < MCP::Tool
      description <<~DESC
        Search memories using full-text search.

        AUTOMATIC TRIGGERING - Use when user:

        Asks About Past Decisions:
        - "What did we decide about...?"
        - "Why did we choose...?"
        - "What was the reasoning for...?"

        References Previous Work:
        - "Last time we..."
        - "Previously, we..."
        - "Remember when we..."

        Asks Implementation Questions:
        - "How did we implement...?"
        - "What approach did we use for...?"

        Troubleshooting:
        - "Have we seen this error before?"
        - "Is there a known workaround?"

        Trigger Words (search immediately):
        - Past tense: "decided", "implemented", "discussed"
        - Time references: "yesterday", "last week", "previously"
        - Memory references: "remember", "recall", "mentioned"

        IMPORTANT: Search proactively when user references past work.
      DESC

      input_schema(
        properties: {
          query: {
            type: 'string',
            description: 'Search query'
          },
          project: {
            type: 'string',
            description: 'Limit search to specific project (omit to search all)'
          },
          memory_type: {
            type: 'string',
            enum: %w[note decision pattern bug learning],
            description: 'Filter by memory type'
          },
          limit: {
            type: 'integer',
            description: 'Maximum results (default: 10)',
            default: 10
          }
        },
        required: ['query']
      )

      class << self
        def call(query:, project: nil, memory_type: nil, limit: 10, server_context:)
          db_manager = server_context[:db_manager]

          results = db_manager.search_all(
            query,
            project: project,
            memory_type: memory_type,
            limit: limit
          )

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              results: results,
              count: results.length,
              query: query
            })
          }])
        end
      end
    end
  end
end
```

### 7. Get Project Context Tool (`lib/recollect/tools/get_context.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class GetContext < MCP::Tool
      description <<~DESC
        Get comprehensive context for a project.

        AUTOMATIC TRIGGERING - Use this tool:

        At Session Start:
        - User mentions a project name
        - User says "let's work on X project"

        When Switching Context:
        - User changes to different project
        - User asks "what are we working on?"

        For Status Updates:
        - User asks about project status
        - User wants overview of decisions

        This is your "load project state" tool. Use it liberally
        when starting work on any named project.
      DESC

      input_schema(
        properties: {
          project: {
            type: 'string',
            description: 'Project name'
          }
        },
        required: ['project']
      )

      class << self
        def call(project:, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          memories = db.list(limit: 100)
          by_type = memories.group_by { |m| m['memory_type'] }

          # Get recent (last 7 days)
          cutoff = (Time.now - 7 * 24 * 60 * 60).strftime('%Y-%m-%dT%H:%M:%SZ')
          recent = memories.select { |m| m['created_at'] > cutoff }

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              project: project,
              total_memories: memories.length,
              recent_count: recent.length,
              by_type: by_type.transform_values(&:length),
              recent_memories: recent.take(20)
            })
          }])
        end
      end
    end
  end
end
```

### 8. List Projects Tool (`lib/recollect/tools/list_projects.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class ListProjects < MCP::Tool
      description 'List all projects that have stored memories'

      input_schema(properties: {})

      class << self
        def call(server_context:)
          db_manager = server_context[:db_manager]
          projects = db_manager.list_projects

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              projects: projects,
              count: projects.length
            })
          }])
        end
      end
    end
  end
end
```

### 9. Delete Memory Tool (`lib/recollect/tools/delete_memory.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module Tools
    class DeleteMemory < MCP::Tool
      description 'Delete a specific memory by ID'

      input_schema(
        properties: {
          id: {
            type: 'integer',
            description: 'Memory ID to delete'
          },
          project: {
            type: 'string',
            description: 'Project name (omit for global)'
          }
        },
        required: ['id']
      )

      class << self
        def call(id:, project: nil, server_context:)
          db_manager = server_context[:db_manager]
          db = db_manager.get_database(project)

          success = db.delete(id)

          MCP::Tool::Response.new([{
            type: 'text',
            text: JSON.generate({
              success: success,
              deleted_id: success ? id : nil,
              message: success ? 'Memory deleted' : 'Memory not found'
            })
          }])
        end
      end
    end
  end
end
```

---

## MCP Server Factory

### 10. MCP Server (`lib/recollect/mcp_server.rb`)

```ruby
# frozen_string_literal: true

require 'mcp'

module Recollect
  module MCPServer
    TOOLS = [
      Tools::StoreMemory,
      Tools::SearchMemory,
      Tools::GetContext,
      Tools::ListProjects,
      Tools::DeleteMemory
    ].freeze

    class << self
      def build(db_manager)
        MCP::Server.new(
          name: 'recollect',
          version: Recollect::VERSION,
          tools: TOOLS,
          server_context: { db_manager: db_manager }
        )
      end
    end
  end
end
```

---

## HTTP Server

### 11. Sinatra Application (`lib/recollect/http_server.rb`)

```ruby
# frozen_string_literal: true

require 'sinatra/base'
require 'json'

module Recollect
  class HTTPServer < Sinatra::Base
    configure do
      set :public_folder, proc { Recollect.root.join('public') }
      set :views, proc { Recollect.root.join('views') }
      set :show_exceptions, :after_handler
    end

    helpers do
      def db_manager
        @db_manager ||= DatabaseManager.new(Recollect.config)
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
        halt 400, json_response({ error: 'Invalid JSON' }, status_code: 400)
      end
    end

    # Health check
    get '/health' do
      json_response({ status: 'healthy', version: Recollect::VERSION })
    end

    # MCP endpoint
    post '/mcp' do
      request.body.rewind
      body = request.body.read
      content_type :json
      mcp_server.handle_json(body)
    end

    # ========== REST API ==========

    # List memories
    get '/api/memories' do
      project = params['project']
      memory_type = params['type']
      limit = (params['limit'] || 50).to_i
      offset = (params['offset'] || 0).to_i

      db = db_manager.get_database(project)
      memories = db.list(memory_type: memory_type, limit: limit, offset: offset)
      memories.each { |m| m['project'] = project }

      json_response(memories)
    end

    # Search memories
    get '/api/memories/search' do
      query = params['q']
      halt 400, json_response({ error: 'Query parameter "q" required' }, status_code: 400) unless query

      results = db_manager.search_all(
        query,
        project: params['project'],
        memory_type: params['type'],
        limit: (params['limit'] || 10).to_i
      )

      json_response({ results: results, count: results.length, query: query })
    end

    # Get single memory
    get '/api/memories/:id' do
      project = params['project']
      db = db_manager.get_database(project)

      memory = db.get(params['id'].to_i)
      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless memory

      memory['project'] = project
      json_response(memory)
    end

    # Create memory
    post '/api/memories' do
      data = parse_json_body
      project = data['project']
      db = db_manager.get_database(project)

      id = db.store(
        content: data['content'],
        memory_type: data['memory_type'] || 'note',
        tags: data['tags'],
        metadata: data['metadata'],
        source: 'api'
      )

      memory = db.get(id)
      memory['project'] = project

      status 201
      json_response(memory)
    end

    # Update memory
    put '/api/memories/:id' do
      data = parse_json_body
      project = data['project']
      db = db_manager.get_database(project)

      success = db.update(
        params['id'].to_i,
        content: data['content'],
        tags: data['tags'],
        metadata: data['metadata']
      )

      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless success

      memory = db.get(params['id'].to_i)
      memory['project'] = project
      json_response(memory)
    end

    # Delete memory
    delete '/api/memories/:id' do
      project = params['project']
      db = db_manager.get_database(project)

      success = db.delete(params['id'].to_i)
      halt 404, json_response({ error: 'Memory not found' }, status_code: 404) unless success

      json_response({ deleted: params['id'].to_i })
    end

    # List projects
    get '/api/projects' do
      projects = db_manager.list_projects
      json_response({ projects: projects, count: projects.length })
    end

    # Serve Web UI
    get '/' do
      send_file File.join(settings.public_folder, 'index.html')
    end

    # Error handling
    error do
      json_response({ error: env['sinatra.error'].message }, status_code: 500)
    end
  end
end
```

---

## Configuration Files

### 12. Rack Config (`config.ru`)

```ruby
# frozen_string_literal: true

require_relative 'lib/recollect'
require 'rack/cors'

use Rack::Cors do
  allow do
    origins '*'
    resource '*', headers: :any, methods: %i[get post put delete options]
  end
end

run Recollect::HTTPServer
```

### 13. Puma Config (`config/puma.rb`)

```ruby
# frozen_string_literal: true

# Workers (processes)
workers ENV.fetch('WEB_CONCURRENCY', 2).to_i

# Threads per worker
threads_count = ENV.fetch('PUMA_MAX_THREADS', 5).to_i
threads threads_count, threads_count

# Environment
environment ENV.fetch('RACK_ENV', 'development')

# Binding
bind "tcp://#{ENV.fetch('RECOLLECT_HOST', '127.0.0.1')}:#{ENV.fetch('RECOLLECT_PORT', '8080')}"

# Preload app for copy-on-write memory savings
preload_app!

# Lifecycle hooks
on_worker_boot do
  # Each worker gets fresh DB connections via lazy initialization
end

# Allow puma to be restarted by `bin/puma --restart`
plugin :tmp_restart
```

### 14. Rakefile (`Rakefile`)

```ruby
# frozen_string_literal: true

require 'rake/testtask'

Rake::TestTask.new(:test) do |t|
  t.libs << 'test'
  t.libs << 'lib'
  t.test_files = FileList['test/**/*_test.rb']
  t.warning = false
end

task default: :test
```

### 15. Version (`lib/recollect/version.rb`)

```ruby
# frozen_string_literal: true

module Recollect
  VERSION = '0.1.0'
end
```

---

## CLI Tool

### 16. CLI (`bin/recollect`)

```ruby
#!/usr/bin/env ruby
# frozen_string_literal: true

require 'bundler/setup'
require 'thor'
require 'tty-table'
require 'pastel'
require 'net/http'
require 'json'

module Recollect
  class CLI < Thor
    def initialize(*)
      super
      @pastel = Pastel.new
      @base_url = ENV.fetch('RECOLLECT_URL', 'http://localhost:8080')
    end

    desc 'store CONTENT', 'Store a new memory'
    option :project, aliases: '-p', desc: 'Project name'
    option :type, aliases: '-t', default: 'note', desc: 'Memory type'
    option :tags, aliases: '-T', type: :array, desc: 'Tags'
    def store(content)
      data = {
        content: content,
        memory_type: options[:type],
        project: options[:project],
        tags: options[:tags]
      }.compact

      response = post('/api/memories', data)

      if response.code == '201'
        result = JSON.parse(response.body)
        say @pastel.green("Stored memory ##{result['id']}")
        location = result['project'] ? "project '#{result['project']}'" : 'global'
        say "  Location: #{location}"
      else
        say @pastel.red("Error: #{response.message}")
      end
    rescue StandardError => e
      say @pastel.red("Error: #{e.message}")
    end

    desc 'search QUERY', 'Search memories'
    option :project, aliases: '-p', desc: 'Limit to project'
    option :type, aliases: '-t', desc: 'Filter by type'
    option :limit, aliases: '-l', type: :numeric, default: 10
    def search(query)
      params = {
        q: query,
        project: options[:project],
        type: options[:type],
        limit: options[:limit]
      }.compact

      response = get('/api/memories/search', params)

      if response.code == '200'
        data = JSON.parse(response.body)
        display_memories(data['results'])
        say "\n#{data['count']} results for '#{query}'"
      else
        say @pastel.red("Error: #{response.message}")
      end
    rescue StandardError => e
      say @pastel.red("Error: #{e.message}")
    end

    desc 'list', 'List recent memories'
    option :project, aliases: '-p', desc: 'Filter by project'
    option :type, aliases: '-t', desc: 'Filter by type'
    option :limit, aliases: '-l', type: :numeric, default: 20
    def list
      params = {
        project: options[:project],
        type: options[:type],
        limit: options[:limit]
      }.compact

      response = get('/api/memories', params)

      if response.code == '200'
        memories = JSON.parse(response.body)
        display_memories(memories)
      else
        say @pastel.red("Error: #{response.message}")
      end
    rescue StandardError => e
      say @pastel.red("Error: #{e.message}")
    end

    desc 'projects', 'List all projects'
    def projects
      response = get('/api/projects')

      if response.code == '200'
        data = JSON.parse(response.body)

        if data['projects'].empty?
          say @pastel.yellow('No projects found.')
        else
          say "\nProjects:"
          data['projects'].each { |p| say "  - #{p}" }
          say "\nTotal: #{data['count']}"
        end
      else
        say @pastel.red("Error: #{response.message}")
      end
    rescue StandardError => e
      say @pastel.red("Error: #{e.message}")
    end

    desc 'status', 'Check server status'
    def status
      response = get('/health')

      if response.code == '200'
        data = JSON.parse(response.body)
        say @pastel.green("Server is running (v#{data['version']})")
      else
        say @pastel.red('Server is not responding')
      end
    rescue StandardError => e
      say @pastel.red("Server unreachable: #{e.message}")
    end

    private

    def get(path, params = {})
      uri = URI("#{@base_url}#{path}")
      uri.query = URI.encode_www_form(params) unless params.empty?
      Net::HTTP.get_response(uri)
    end

    def post(path, data)
      uri = URI("#{@base_url}#{path}")
      http = Net::HTTP.new(uri.host, uri.port)
      request = Net::HTTP::Post.new(uri.path, 'Content-Type' => 'application/json')
      request.body = data.to_json
      http.request(request)
    end

    def display_memories(memories)
      if memories.empty?
        say @pastel.yellow('No memories found.')
        return
      end

      table = TTY::Table.new(
        header: %w[ID Type Project Content],
        rows: memories.map do |m|
          [
            m['id'],
            m['memory_type'],
            m['project'] || 'global',
            truncate(m['content'], 50)
          ]
        end
      )

      puts table.render(:unicode)
    end

    def truncate(text, length)
      text.length > length ? "#{text[0...length]}..." : text
    end
  end
end

Recollect::CLI.start(ARGV)
```

### 17. Server Executable (`bin/server`)

```ruby
#!/usr/bin/env ruby
# frozen_string_literal: true

require 'bundler/setup'

exec 'bundle', 'exec', 'puma', '-C', 'config/puma.rb'
```

Make executables:
```bash
chmod +x bin/server bin/recollect
```

---

## Web UI

### 18. HTML (`public/index.html`)

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Recollect</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <div class="container">
    <header>
      <h1>Recollect</h1>
      <div class="controls">
        <select id="projectFilter">
          <option value="">All Projects</option>
        </select>
        <input type="search" id="searchBox" placeholder="Search memories...">
        <button id="searchBtn">Search</button>
      </div>
    </header>

    <main id="memoriesList"></main>
  </div>

  <script src="/app.js"></script>
</body>
</html>
```

### 19. CSS (`public/style.css`)

```css
* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  background: #f5f5f5;
  color: #333;
  line-height: 1.6;
}

.container {
  max-width: 900px;
  margin: 0 auto;
  padding: 20px;
}

header {
  margin-bottom: 30px;
}

header h1 {
  margin-bottom: 20px;
  color: #2c3e50;
}

.controls {
  display: flex;
  gap: 10px;
}

.controls select,
.controls input {
  padding: 10px 15px;
  border: 1px solid #ddd;
  border-radius: 5px;
  font-size: 14px;
}

.controls input {
  flex: 1;
}

.controls button {
  padding: 10px 20px;
  background: #3498db;
  color: white;
  border: none;
  border-radius: 5px;
  cursor: pointer;
}

.controls button:hover {
  background: #2980b9;
}

.memory-card {
  background: white;
  border-radius: 8px;
  padding: 20px;
  margin-bottom: 15px;
  box-shadow: 0 2px 4px rgba(0,0,0,0.1);
}

.memory-header {
  display: flex;
  gap: 10px;
  margin-bottom: 10px;
  font-size: 12px;
}

.type {
  padding: 3px 8px;
  border-radius: 3px;
  background: #ecf0f1;
  font-weight: 500;
}

.type-decision { background: #3498db; color: white; }
.type-pattern { background: #2ecc71; color: white; }
.type-bug { background: #e74c3c; color: white; }
.type-learning { background: #9b59b6; color: white; }

.project {
  color: #7f8c8d;
}

.date {
  color: #95a5a6;
  margin-left: auto;
}

.content {
  white-space: pre-wrap;
}

.empty {
  text-align: center;
  color: #95a5a6;
  padding: 40px;
}
```

### 19. JavaScript (`public/app.js`)

```javascript
const API = '/api';

async function loadMemories(project = null, query = null) {
  const params = new URLSearchParams();
  if (project) params.append('project', project);
  if (query) params.append('q', query);

  const endpoint = query
    ? `${API}/memories/search?${params}`
    : `${API}/memories?${params}`;

  try {
    const resp = await fetch(endpoint);
    const data = await resp.json();
    const memories = query ? data.results : data;
    displayMemories(memories);
  } catch (err) {
    console.error('Failed to load memories:', err);
  }
}

function displayMemories(memories) {
  const container = document.getElementById('memoriesList');
  container.innerHTML = '';

  if (!memories || memories.length === 0) {
    container.innerHTML = '<p class="empty">No memories found</p>';
    return;
  }

  memories.forEach(mem => {
    const card = document.createElement('div');
    card.className = 'memory-card';
    card.innerHTML = `
      <div class="memory-header">
        <span class="type type-${mem.memory_type}">${mem.memory_type}</span>
        <span class="project">${mem.project || 'global'}</span>
        <span class="date">${formatDate(mem.created_at)}</span>
      </div>
      <div class="content">${escapeHtml(mem.content)}</div>
    `;
    container.appendChild(card);
  });
}

async function loadProjects() {
  try {
    const resp = await fetch(`${API}/projects`);
    const data = await resp.json();
    const select = document.getElementById('projectFilter');

    data.projects.forEach(proj => {
      const option = document.createElement('option');
      option.value = proj;
      option.textContent = proj;
      select.appendChild(option);
    });
  } catch (err) {
    console.error('Failed to load projects:', err);
  }
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString();
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// Event listeners
document.addEventListener('DOMContentLoaded', () => {
  loadProjects();
  loadMemories();

  document.getElementById('searchBtn').addEventListener('click', () => {
    const query = document.getElementById('searchBox').value;
    const project = document.getElementById('projectFilter').value;
    loadMemories(project || null, query || null);
  });

  document.getElementById('searchBox').addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
      document.getElementById('searchBtn').click();
    }
  });

  document.getElementById('projectFilter').addEventListener('change', () => {
    const project = document.getElementById('projectFilter').value;
    loadMemories(project || null, null);
  });
});
```

---

## Testing

### 20. Test Helper (`test/test_helper.rb`)

```ruby
# frozen_string_literal: true

ENV['RACK_ENV'] = 'test'
ENV['RECOLLECT_DATA_DIR'] = File.join(__dir__, 'tmp', 'test_data')

require 'bundler/setup'
require 'recollect'
require 'minitest/autorun'
require 'rack/test'
require 'fileutils'

# Ensure test data directory exists
FileUtils.mkdir_p(ENV['RECOLLECT_DATA_DIR'])

module Recollect
  class TestCase < Minitest::Test
    include Rack::Test::Methods

    def setup
      # Clean databases between tests
      Dir.glob(File.join(ENV['RECOLLECT_DATA_DIR'], '**/*.db')).each do |f|
        FileUtils.rm_f(f)
      end
    end

    def teardown
      # Subclasses can override
    end
  end
end

Minitest.after_run do
  FileUtils.rm_rf(ENV['RECOLLECT_DATA_DIR'])
end
```

### 21. Database Test (`test/recollect/database_test.rb`)

```ruby
# frozen_string_literal: true

require 'test_helper'

class DatabaseTest < Recollect::TestCase
  def setup
    super
    @db_path = Pathname.new(ENV['RECOLLECT_DATA_DIR']).join('test.db')
    @db = Recollect::Database.new(@db_path)
  end

  def teardown
    @db.close
    super
  end

  def test_store_returns_id
    id = @db.store(content: 'Test memory')
    assert id > 0
  end

  def test_store_with_all_attributes
    id = @db.store(
      content: 'Test',
      memory_type: 'decision',
      tags: %w[ruby test],
      metadata: { key: 'value' }
    )

    memory = @db.get(id)
    assert_equal 'decision', memory['memory_type']
    assert_equal %w[ruby test], memory['tags']
    assert_equal({ 'key' => 'value' }, memory['metadata'])
  end

  def test_search_finds_by_content
    @db.store(content: 'Ruby async patterns', memory_type: 'pattern')
    @db.store(content: 'Python async/await', memory_type: 'note')
    @db.store(content: 'JavaScript promises', memory_type: 'note')

    results = @db.search('async')
    assert_equal 2, results.length
  end

  def test_search_filters_by_type
    @db.store(content: 'Ruby async patterns', memory_type: 'pattern')
    @db.store(content: 'Python async/await', memory_type: 'note')

    results = @db.search('async', memory_type: 'pattern')
    assert_equal 1, results.length
    assert_includes results.first['content'], 'Ruby'
  end

  def test_list_returns_in_descending_order
    3.times { |i| @db.store(content: "Memory #{i}") }

    results = @db.list(limit: 2)
    assert_equal 2, results.length
    assert_equal 'Memory 2', results.first['content']
  end

  def test_update_changes_content
    id = @db.store(content: 'Original')
    @db.update(id, content: 'Updated')

    memory = @db.get(id)
    assert_equal 'Updated', memory['content']
  end

  def test_delete_removes_memory
    id = @db.store(content: 'To delete')
    assert @db.delete(id)
    assert_nil @db.get(id)
  end
end
```

---

## Usage

### Starting the Server

```bash
# Development
./bin/server

# Or directly with Puma
bundle exec puma -C config/puma.rb

# With custom settings
RECOLLECT_HOST=0.0.0.0 RECOLLECT_PORT=9000 ./bin/server
```

### CLI Usage

```bash
# Store a memory
./bin/recollect store "We use Puma for threading" --project myapp --type pattern

# Search
./bin/recollect search "threading"

# List recent
./bin/recollect list --limit 20

# Check status
./bin/recollect status
```

### Configure Claude Code

Add to your Claude Code MCP configuration:

```json
{
  "mcpServers": {
    "recollect": {
      "type": "http",
      "url": "http://localhost:8080/mcp"
    }
  }
}
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `8080` | Server port |
| `WEB_CONCURRENCY` | `2` | Puma worker processes |
| `PUMA_MAX_THREADS` | `5` | Threads per worker |
| `RACK_ENV` | `development` | Environment |
