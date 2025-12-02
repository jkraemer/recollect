# SQLite-Vec Integration Implementation Plan

## Overview

Add semantic vector search to Recollect using sqlite-vec extension and local Python embeddings. Hybrid search combines FTS5 keyword matching with vector similarity for better results.

**Key Design Decisions:**
- **Opt-in feature**: Controlled by `RECOLLECT_ENABLE_VECTORS=true` environment variable
- **Local embeddings**: Python CLI script using sentence-transformers (no cloud APIs)
- **Out-of-band processing**: Background worker thread with queue for embedding generation
- **Graceful degradation**: Everything works without Python/sqlite-vec installed
- **Automatic hybrid search**: When vectors available, search uses both FTS5 + vectors transparently

**Architecture:**
```
┌─────────────────────────────────────────────────────────┐
│                   Recollect Server                      │
├─────────────────────────────────────────────────────────┤
│  Store Memory → Queue → Worker Thread → Python CLI      │
│                              ↓                          │
│                     sqlite-vec table                    │
├─────────────────────────────────────────────────────────┤
│  Search → FTS5 results + Vector results → Hybrid merge  │
└─────────────────────────────────────────────────────────┘
```

## Prerequisites (User Responsibility)

1. **sqlite-vec extension** installed at `~/.local/lib/sqlite-vec/vec0.so`
2. **Python 3.8+** with sentence-transformers:
   ```bash
   pip install sentence-transformers
   ```

## Implementation Steps

### Phase 1: Python Embedding Script

**File**: `bin/embed`

Create a Python CLI script that:
- Reads JSON from stdin: `{"texts": ["text1", "text2"]}`
- Outputs JSON to stdout: `{"embeddings": [[0.1, 0.2, ...], [...]], "dimensions": 384}`
- Uses `all-MiniLM-L6-v2` model (384 dimensions)
- Caches model after first load (subsequent calls in same process are fast)

```python
#!/usr/bin/env python3
"""Generate embeddings for Recollect memory server."""
import sys
import json
from sentence_transformers import SentenceTransformer

MODEL_NAME = "sentence-transformers/all-MiniLM-L6-v2"
DIMENSIONS = 384

def main():
    model = SentenceTransformer(MODEL_NAME)

    input_data = json.load(sys.stdin)
    texts = input_data.get("texts", [])

    if not texts:
        json.dump({"embeddings": [], "dimensions": DIMENSIONS}, sys.stdout)
        return

    embeddings = model.encode(texts, normalize_embeddings=True)

    json.dump({
        "embeddings": [emb.tolist() for emb in embeddings],
        "dimensions": DIMENSIONS
    }, sys.stdout)

if __name__ == "__main__":
    main()
```

**Tests:**
- Script outputs valid JSON for single text
- Script outputs valid JSON for batch of texts
- Script handles empty input gracefully
- Embeddings are normalized (L2 norm ≈ 1.0)

---

### Phase 2: Configuration Updates

**File**: `lib/recollect/config.rb`

Add configuration for vector search:

```ruby
attr_accessor :enable_vectors, :vector_dimensions, :embed_script_path

def initialize
  # ... existing config ...

  @enable_vectors = ENV.fetch('RECOLLECT_ENABLE_VECTORS', 'false') == 'true'
  @vector_dimensions = 384  # all-MiniLM-L6-v2
  @embed_script_path = Recollect.root.join('bin', 'embed')
end

def vec_extension_path
  paths = [
    '~/.local/lib/sqlite-vec/vec0.so',
    '~/.local/lib/sqlite-vec/vec0.dylib',
    '/usr/local/lib/vec0.so'
  ]

  paths.each do |path|
    expanded = File.expand_path(path)
    return expanded if File.exist?(expanded)
  end

  nil
end

def vectors_available?
  enable_vectors && vec_extension_path && File.executable?(embed_script_path)
end
```

**Tests:**
- `vectors_available?` returns false when RECOLLECT_ENABLE_VECTORS not set
- `vectors_available?` returns false when extension missing
- `vectors_available?` returns false when embed script missing
- `vec_extension_path` finds extension in standard locations

---

### Phase 3: Embedding Client

**File**: `lib/recollect/embedding_client.rb`

Ruby client that shells out to Python script:

```ruby
module Recollect
  class EmbeddingClient
    class EmbeddingError < StandardError; end

    TIMEOUT = 60  # seconds (first call loads model)

    def initialize(script_path: Recollect.config.embed_script_path)
      @script_path = script_path.to_s
    end

    def embed(text)
      embed_batch([text]).first
    end

    def embed_batch(texts)
      return [] if texts.empty?

      input = JSON.generate({ texts: texts })
      output, status = Open3.capture2(
        'python3', @script_path,
        stdin_data: input,
        timeout: TIMEOUT
      )

      unless status.success?
        raise EmbeddingError, "Embedding script failed: exit #{status.exitstatus}"
      end

      result = JSON.parse(output)
      result['embeddings']
    rescue JSON::ParserError => e
      raise EmbeddingError, "Invalid embedding output: #{e.message}"
    rescue Timeout::Error
      raise EmbeddingError, "Embedding script timed out after #{TIMEOUT}s"
    end

    def healthy?
      embed_batch(['test'])
      true
    rescue EmbeddingError
      false
    end
  end
end
```

**Tests:**
- `embed` returns array of floats with correct dimensions
- `embed_batch` processes multiple texts
- `embed_batch` returns empty array for empty input
- `EmbeddingError` raised on script failure
- `EmbeddingError` raised on invalid JSON output
- `healthy?` returns true when script works
- `healthy?` returns false when script fails

---

### Phase 4: Embedding Queue & Worker

**File**: `lib/recollect/embedding_worker.rb`

Background worker that processes embedding jobs from a queue:

```ruby
module Recollect
  class EmbeddingWorker
    BATCH_SIZE = 10
    BATCH_WAIT = 2  # seconds to wait for batch to fill

    def initialize(db_manager)
      @db_manager = db_manager
      @queue = Queue.new
      @running = false
      @thread = nil
      @client = EmbeddingClient.new
    end

    def start
      return if @running
      @running = true
      @thread = Thread.new { run_loop }
    end

    def stop
      @running = false
      @queue.close
      @thread&.join(5)
    end

    def enqueue(memory_id:, content:, project:)
      return unless @running
      @queue << { memory_id: memory_id, content: content, project: project }
    end

    private

    def run_loop
      while @running
        batch = collect_batch
        process_batch(batch) unless batch.empty?
      end
    end

    def collect_batch
      batch = []
      deadline = Time.now + BATCH_WAIT

      while batch.size < BATCH_SIZE && Time.now < deadline
        begin
          item = @queue.pop(true)  # non-blocking
          batch << item
        rescue ThreadError
          sleep 0.1
        end
      end

      batch
    end

    def process_batch(batch)
      texts = batch.map { |item| item[:content] }

      embeddings = @client.embed_batch(texts)

      batch.zip(embeddings).each do |item, embedding|
        store_embedding(item, embedding)
      end
    rescue EmbeddingClient::EmbeddingError => e
      warn "[EmbeddingWorker] Batch failed: #{e.message}"
    end

    def store_embedding(item, embedding)
      db = @db_manager.get_database(item[:project])
      db.store_embedding(item[:memory_id], embedding)
    rescue => e
      warn "[EmbeddingWorker] Failed to store embedding for ##{item[:memory_id]}: #{e.message}"
    end
  end
end
```

**Tests:**
- Worker processes single item from queue
- Worker batches multiple items together
- Worker handles embedding errors gracefully (logs, continues)
- Worker stops cleanly when `stop` called
- Queue items processed in order

---

### Phase 5: Database Vector Support

**File**: `lib/recollect/database.rb`

Add vector table and operations to Database class:

#### 5.1 Schema Addition

```ruby
VECTOR_SCHEMA = <<~SQL
  CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
    embedding float[384]
  );
SQL
```

#### 5.2 Extension Loading

```ruby
def initialize(db_path, load_vectors: false)
  @db_path = db_path.to_s
  @db = SQLite3::Database.new(@db_path)
  @db.results_as_hash = true
  @vectors_enabled = false

  configure_database
  create_schema

  load_vector_extension if load_vectors
end

def load_vector_extension
  vec_path = Recollect.config.vec_extension_path
  return unless vec_path

  @db.enable_load_extension(true)
  @db.load_extension(vec_path)
  @db.enable_load_extension(false)

  @db.execute_batch(VECTOR_SCHEMA)
  @vectors_enabled = true
rescue SQLite3::Exception => e
  warn "[Database] Failed to load sqlite-vec: #{e.message}"
  @vectors_enabled = false
end

def vectors_enabled?
  @vectors_enabled
end
```

#### 5.3 Store Embedding

```ruby
def store_embedding(memory_id, embedding)
  return unless @vectors_enabled

  embedding_blob = embedding.pack('e*')  # little-endian float

  @db.execute(<<~SQL, [memory_id, embedding_blob])
    INSERT OR REPLACE INTO vec_memories(rowid, embedding)
    VALUES (?, ?)
  SQL
end
```

#### 5.4 Vector Search

```ruby
def vector_search(query_embedding, limit: 10)
  return [] unless @vectors_enabled

  query_blob = query_embedding.pack('e*')

  sql = <<~SQL
    SELECT
      m.*,
      v.distance
    FROM vec_memories v
    JOIN memories m ON m.id = v.rowid
    WHERE v.embedding MATCH ?
      AND k = ?
    ORDER BY v.distance
  SQL

  @db.execute(sql, [query_blob, limit]).map { |row| deserialize(row) }
end

def embedding_count
  return 0 unless @vectors_enabled
  @db.get_first_value('SELECT COUNT(*) FROM vec_memories') || 0
end

def memories_without_embeddings(limit: 100)
  return [] unless @vectors_enabled

  sql = <<~SQL
    SELECT id, content FROM memories
    WHERE id NOT IN (SELECT rowid FROM vec_memories)
    ORDER BY created_at DESC
    LIMIT ?
  SQL

  @db.execute(sql, [limit])
end
```

**Tests:**
- `load_vector_extension` succeeds when extension available
- `load_vector_extension` fails gracefully when extension missing
- `vectors_enabled?` reflects actual state
- `store_embedding` stores embedding correctly
- `store_embedding` no-op when vectors disabled
- `vector_search` returns results with distance
- `vector_search` returns empty array when vectors disabled
- `embedding_count` returns correct count
- `memories_without_embeddings` finds memories needing embeddings

---

### Phase 6: Database Manager Updates

**File**: `lib/recollect/database_manager.rb`

Update to manage embedding worker and pass vector config to databases:

```ruby
def initialize(config = Recollect.config)
  @config = config
  @databases = {}
  @mutex = Mutex.new
  @embedding_worker = nil

  start_embedding_worker if @config.vectors_available?
end

def get_database(project = nil)
  key = project || :global

  @mutex.synchronize do
    @databases[key] ||= begin
      path = project ? @config.project_db_path(project) : @config.global_db_path
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

def hybrid_search(query, project: nil, memory_type: nil, limit: 10)
  # If vectors not available, fall back to FTS5 only
  unless @config.vectors_available? && vectors_ready?
    return search_all(query, project: project, memory_type: memory_type, limit: limit)
  end

  # Get query embedding
  embedding = EmbeddingClient.new.embed(query)

  # Collect results from both methods
  fts_results = search_all(query, project: project, memory_type: memory_type, limit: limit * 2)
  vec_results = vector_search_all(embedding, project: project, limit: limit * 2)

  # Merge and rank
  merge_hybrid_results(fts_results, vec_results, limit)
end

private

def start_embedding_worker
  @embedding_worker = EmbeddingWorker.new(self)
  @embedding_worker.start
end

def vectors_ready?
  # Check if at least one database has vectors enabled
  @databases.values.any?(&:vectors_enabled?)
end

def vector_search_all(embedding, project: nil, limit: 10)
  if project
    db = get_database(project)
    results = db.vector_search(embedding, limit: limit)
    results.each { |m| m['project'] = project }
    results
  else
    results = []

    # Search global
    global_results = get_database(nil).vector_search(embedding, limit: limit)
    global_results.each { |m| m['project'] = nil }
    results.concat(global_results)

    # Search all projects
    list_projects.each do |proj|
      proj_results = get_database(proj).vector_search(embedding, limit: limit)
      proj_results.each { |m| m['project'] = proj }
      results.concat(proj_results)
    end

    results
  end
end

def merge_hybrid_results(fts_results, vec_results, limit)
  scores = {}

  # Score FTS results (weight: 0.6)
  max_fts_rank = fts_results.map { |m| (m['rank'] || 0).abs }.max || 1.0
  fts_results.each do |mem|
    normalized = (mem['rank'] || 0).abs / max_fts_rank
    scores[mem['id']] = {
      memory: mem,
      fts_score: normalized,
      vec_score: 0.0
    }
  end

  # Score vector results (weight: 0.4)
  max_distance = vec_results.map { |m| m['distance'] || 0 }.max || 1.0
  vec_results.each do |mem|
    # Lower distance = better match, invert to score
    normalized = 1.0 - ((mem['distance'] || 0) / max_distance)

    if scores[mem['id']]
      scores[mem['id']][:vec_score] = normalized
    else
      scores[mem['id']] = {
        memory: mem,
        fts_score: 0.0,
        vec_score: normalized
      }
    end
  end

  # Combine scores and sort
  scores.values
    .map do |entry|
      combined = (entry[:fts_score] * 0.6) + (entry[:vec_score] * 0.4)
      entry[:memory].merge('combined_score' => combined)
    end
    .sort_by { |m| -m['combined_score'] }
    .take(limit)
end
```

**Tests:**
- `store_with_embedding` queues embedding job when vectors enabled
- `store_with_embedding` works normally when vectors disabled
- `hybrid_search` falls back to FTS5 when vectors unavailable
- `hybrid_search` combines FTS5 and vector results
- `merge_hybrid_results` ranks correctly (items in both lists score higher)
- Worker is started only when vectors available
- `close_all` stops embedding worker

---

### Phase 7: Update Store Memory Tool

**File**: `lib/recollect/tools/store_memory.rb`

Use new `store_with_embedding` method:

```ruby
class << self
  def call(content:, memory_type: 'note', tags: nil, project: nil, server_context:)
    db_manager = server_context[:db_manager]

    id = db_manager.store_with_embedding(
      project: project,
      content: content,
      memory_type: memory_type,
      tags: tags,
      metadata: nil,
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
```

**Tests:**
- Store tool uses `store_with_embedding`
- Embedding is queued for new memories

---

### Phase 8: Update Search Memory Tool

**File**: `lib/recollect/tools/search_memory.rb`

Use hybrid search automatically:

```ruby
class << self
  def call(query:, server_context:, project: nil, memory_type: nil, tags: nil, limit: 10)
    db_manager = server_context[:db_manager]

    results = if tags && !tags.empty?
      # Tag search doesn't use vectors (yet)
      db_manager.search_by_tags(tags, project: project, memory_type: memory_type, limit: limit)
    else
      # Use hybrid search (auto-falls back to FTS5 if vectors unavailable)
      db_manager.hybrid_search(query, project: project, memory_type: memory_type, limit: limit)
    end

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
```

**Tests:**
- Search uses hybrid_search for text queries
- Search still uses search_by_tags for tag filtering
- Results include combined_score when vectors used

---

### Phase 9: HTTP API Updates

**File**: `lib/recollect/http_server.rb`

#### 9.1 Update Search Endpoint

```ruby
get '/api/memories/search' do
  query = params['q']
  halt 400, json_response({ error: 'Query parameter "q" required' }, status_code: 400) unless query

  results = db_manager.hybrid_search(
    query,
    project: params['project'],
    memory_type: params['type'],
    limit: (params['limit'] || 10).to_i
  )

  json_response({ results: results, count: results.length, query: query })
end
```

#### 9.2 Add Vector Status Endpoint

```ruby
get '/api/vectors/status' do
  config = Recollect.config

  if config.vectors_available?
    # Count embeddings across all databases
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

    json_response({
      enabled: true,
      healthy: true,
      total_memories: total_memories,
      total_embeddings: total_embeddings,
      coverage: total_memories > 0 ? (total_embeddings.to_f / total_memories * 100).round(1) : 0
    })
  else
    json_response({
      enabled: false,
      reason: determine_vector_unavailable_reason
    })
  end
end

private

def determine_vector_unavailable_reason
  config = Recollect.config
  return 'RECOLLECT_ENABLE_VECTORS not set' unless config.enable_vectors
  return 'sqlite-vec extension not found' unless config.vec_extension_path
  return 'embed script not found' unless File.exist?(config.embed_script_path)
  'unknown'
end
```

#### 9.3 Add Backfill Endpoint

```ruby
post '/api/vectors/backfill' do
  unless Recollect.config.vectors_available?
    halt 400, json_response({ error: 'Vector search not enabled' }, status_code: 400)
  end

  project = params['project']
  limit = (params['limit'] || 100).to_i

  db = db_manager.get_database(project)
  pending = db.memories_without_embeddings(limit: limit)

  pending.each do |row|
    db_manager.instance_variable_get(:@embedding_worker)&.enqueue(
      memory_id: row['id'],
      content: row['content'],
      project: project
    )
  end

  json_response({
    success: true,
    queued: pending.length,
    message: "Queued #{pending.length} memories for embedding generation"
  })
end
```

**Tests:**
- GET `/api/vectors/status` returns enabled state
- GET `/api/vectors/status` returns coverage percentage
- POST `/api/vectors/backfill` queues pending memories
- POST `/api/vectors/backfill` returns 400 when vectors disabled

---

### Phase 10: CLI Updates

**File**: `bin/recollect`

Add vector-related commands:

```ruby
desc 'vector-status', 'Check vector search status'
def vector_status
  response = get('/api/vectors/status')

  if response.code == '200'
    data = JSON.parse(response.body)

    if data['enabled']
      say @pastel.green('Vector search is enabled')
      say "  Total memories: #{data['total_memories']}"
      say "  With embeddings: #{data['total_embeddings']}"
      say "  Coverage: #{data['coverage']}%"
    else
      say @pastel.yellow('Vector search is not enabled')
      say "  Reason: #{data['reason']}"
    end
  else
    say @pastel.red("Error: #{response.message}")
  end
rescue StandardError => e
  say @pastel.red("Error: #{e.message}")
end

desc 'vector-backfill', 'Generate embeddings for existing memories'
option :project, aliases: '-p', desc: 'Limit to project'
option :limit, aliases: '-l', type: :numeric, default: 100
def vector_backfill
  params = {
    project: options[:project],
    limit: options[:limit]
  }.compact

  response = post_form('/api/vectors/backfill', params)

  if response.code == '200'
    data = JSON.parse(response.body)
    say @pastel.green("Queued #{data['queued']} memories for embedding")
  else
    error = JSON.parse(response.body)['error'] rescue response.message
    say @pastel.red("Error: #{error}")
  end
rescue StandardError => e
  say @pastel.red("Error: #{e.message}")
end

private

def post_form(path, params)
  uri = URI("#{@base_url}#{path}")
  uri.query = URI.encode_www_form(params) unless params.empty?
  Net::HTTP.post_form(uri, params)
end
```

**Tests:**
- `vector-status` displays enabled state
- `vector-status` displays coverage
- `vector-backfill` queues memories
- `vector-backfill` shows error when vectors disabled

---

## Test Strategy

### Unit Tests

1. **EmbeddingClient** - Mock Python script output
2. **EmbeddingWorker** - Test queue processing, batching, error handling
3. **Database** - Test vector operations with/without extension
4. **DatabaseManager** - Test hybrid search merging logic

### Integration Tests

1. **Full flow**: Store memory → embedding generated → search finds it
2. **Graceful degradation**: Everything works when vectors disabled
3. **Backfill**: Existing memories get embeddings

### Manual Testing Checklist

- [ ] `RECOLLECT_ENABLE_VECTORS=false` - server works normally
- [ ] `RECOLLECT_ENABLE_VECTORS=true` without extension - graceful failure
- [ ] `RECOLLECT_ENABLE_VECTORS=true` with extension - vectors enabled
- [ ] Store memory - embedding appears after short delay
- [ ] Search finds semantically similar results
- [ ] `vector-status` shows correct coverage
- [ ] `vector-backfill` processes pending memories

---

## File Summary

| File | Action | Description |
|------|--------|-------------|
| `bin/embed` | Create | Python embedding script |
| `lib/recollect/config.rb` | Modify | Add vector config options |
| `lib/recollect/embedding_client.rb` | Create | Ruby client for Python script |
| `lib/recollect/embedding_worker.rb` | Create | Background queue processor |
| `lib/recollect/database.rb` | Modify | Add vector table + operations |
| `lib/recollect/database_manager.rb` | Modify | Add hybrid search + worker management |
| `lib/recollect/tools/store_memory.rb` | Modify | Use store_with_embedding |
| `lib/recollect/tools/search_memory.rb` | Modify | Use hybrid_search |
| `lib/recollect/http_server.rb` | Modify | Add vector status + backfill endpoints |
| `bin/recollect` | Modify | Add vector CLI commands |
| `test/recollect/embedding_client_test.rb` | Create | Unit tests |
| `test/recollect/embedding_worker_test.rb` | Create | Unit tests |
| `test/recollect/database_vector_test.rb` | Create | Vector-specific tests |

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_ENABLE_VECTORS` | `false` | Enable vector search feature |

---

## Order of Implementation

1. **Phase 1**: Python script (foundation)
2. **Phase 2**: Config updates
3. **Phase 3**: Embedding client
4. **Phase 4**: Worker thread + queue
5. **Phase 5**: Database vector support
6. **Phase 6**: Database manager updates
7. **Phase 7-8**: MCP tool updates
8. **Phase 9**: HTTP API updates
9. **Phase 10**: CLI updates

Each phase should be tested before moving to the next. The system should remain fully functional with `RECOLLECT_ENABLE_VECTORS=false` at all times.
