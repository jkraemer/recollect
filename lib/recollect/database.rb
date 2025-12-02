# frozen_string_literal: true

require "sqlite3"
require "json"
require "date"

module Recollect
  class Database
    VECTOR_SCHEMA = <<~SQL
      CREATE VIRTUAL TABLE IF NOT EXISTS vec_memories USING vec0(
        embedding float[384]
      );
    SQL

    SCHEMA = <<~SQL
      -- Main memories table
      CREATE TABLE IF NOT EXISTS memories (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        content TEXT NOT NULL,
        memory_type TEXT NOT NULL DEFAULT 'note',
        tags TEXT,
        metadata TEXT,
        embedding BLOB,
        created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
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
        INSERT INTO memories_fts(memories_fts, rowid, content, tags, memory_type)
        VALUES('delete', old.id, old.content, old.tags, old.memory_type);
        INSERT INTO memories_fts(rowid, content, tags, memory_type)
        VALUES (new.id, new.content, new.tags, new.memory_type);
      END;
    SQL

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

    def store(content:, memory_type: "note", tags: nil, metadata: nil)
      raise ArgumentError, "content cannot be empty" if content.nil? || content.to_s.strip.empty?

      # Normalize tags to lowercase
      normalized_tags = tags&.map(&:downcase)

      @db.execute(<<~SQL, [content, memory_type, json_encode(normalized_tags), json_encode(metadata)])
        INSERT INTO memories (content, memory_type, tags, metadata)
        VALUES (?, ?, ?, ?)
      SQL
      @db.last_insert_row_id
    end

    def get(id)
      if @vectors_enabled
        sql = <<~SQL
          SELECT m.*, (v.rowid IS NOT NULL) as has_embedding
          FROM memories m
          LEFT JOIN vec_memories v ON v.rowid = m.id
          WHERE m.id = ?
        SQL
        row = @db.get_first_row(sql, id)
      else
        row = @db.get_first_row("SELECT * FROM memories WHERE id = ?", id)
      end
      deserialize(row)
    end

    def search(query, memory_type: nil, limit: 10, offset: 0, created_after: nil, created_before: nil)
      # Handle wildcard query - return all records matching filters
      if query == "*"
        sql = +"SELECT * FROM memories WHERE 1=1"
        params = []

        if memory_type
          types = Array(memory_type)
          placeholders = types.map { "?" }.join(", ")
          sql += " AND memory_type IN (#{placeholders})"
          params.concat(types)
        end

        append_date_filters(sql, params, created_after, created_before)

        sql += " ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        params.push(limit, offset)

        return @db.execute(sql, params).map { |row| deserialize(row) }
      end

      # Build FTS5 query based on input type
      safe_query = if query.is_a?(Array)
        # Array: AND semantics - each term quoted and joined (implicit AND)
        query.map { |term| "\"#{term.gsub('"', '""')}\"" }.join(" ")
      else
        # String: phrase search (existing behavior)
        "\"#{query.gsub('"', '""')}\""
      end

      sql = +<<~SQL
        SELECT memories.*, bm25(memories_fts) as rank
        FROM memories_fts
        JOIN memories ON memories.id = memories_fts.rowid
        WHERE memories_fts MATCH ?
      SQL
      params = [safe_query]

      if memory_type
        types = Array(memory_type)
        placeholders = types.map { "?" }.join(", ")
        sql += " AND memories.memory_type IN (#{placeholders})"
        params.concat(types)
      end

      append_date_filters(sql, params, created_after, created_before, column: "memories.created_at")

      sql += " ORDER BY rank LIMIT ? OFFSET ?"
      params.push(limit, offset)

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def list(memory_type: nil, limit: 50, offset: 0)
      sql = if @vectors_enabled
        +"SELECT m.*, (v.rowid IS NOT NULL) as has_embedding FROM memories m LEFT JOIN vec_memories v ON v.rowid = m.id"
      else
        +"SELECT * FROM memories"
      end
      params = []

      if memory_type
        types = Array(memory_type)
        placeholders = types.map { "?" }.join(", ")
        column = @vectors_enabled ? "m.memory_type" : "memory_type"
        sql << " WHERE #{column} IN (#{placeholders})"
        params.concat(types)
      end

      sql << (@vectors_enabled ? " ORDER BY m.created_at DESC, m.id DESC LIMIT ? OFFSET ?" : " ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?")
      params.push(limit, offset)

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def delete(id) # rubocop:disable Naming/PredicateMethod
      @db.execute("DELETE FROM vec_memories WHERE rowid = ?", id) if @vectors_enabled
      @db.execute("DELETE FROM memories WHERE id = ?", id)
      @db.changes.positive?
    end

    def count(memory_type: nil)
      if memory_type
        @db.get_first_value("SELECT COUNT(*) FROM memories WHERE memory_type = ?", memory_type)
      else
        @db.get_first_value("SELECT COUNT(*) FROM memories")
      end
    end

    def search_by_tags(tag_filters, memory_type: nil, limit: 10, created_after: nil, created_before: nil)
      return [] if tag_filters.nil? || tag_filters.empty?

      # Normalize tag_filters to lowercase
      normalized_tags = tag_filters.map(&:downcase)

      sql = +"SELECT * FROM memories WHERE 1=1"
      params = []

      # Build WHERE clause to match all tags (AND logic)
      normalized_tags.each do |tag|
        sql += " AND tags LIKE ?"
        params << "%\"#{tag}\"%"
      end

      if memory_type
        types = Array(memory_type)
        placeholders = types.map { "?" }.join(", ")
        sql += " AND memory_type IN (#{placeholders})"
        params.concat(types)
      end

      append_date_filters(sql, params, created_after, created_before)

      sql += " ORDER BY created_at DESC, id DESC LIMIT ?"
      params << limit

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def get_tag_stats(memory_type: nil)
      sql = "SELECT tags FROM memories"
      params = []

      if memory_type
        sql += " WHERE memory_type = ?"
        params << memory_type
      end

      rows = @db.execute(sql, params)
      tag_counts = Hash.new(0)

      rows.each do |row|
        tags = json_decode(row["tags"])
        next unless tags

        tags.each do |tag|
          # Normalize to lowercase when counting
          tag_counts[tag.downcase] += 1
        end
      end

      # Sort by frequency descending
      tag_counts.sort_by { |_tag, count| -count }.to_h
    end

    def close
      @db.close
    end

    # Vector search methods

    def store_embedding(memory_id, embedding)
      return unless @vectors_enabled

      embedding_blob = embedding.pack("e*") # little-endian float

      @db.execute(<<~SQL, [memory_id, embedding_blob])
        INSERT OR REPLACE INTO vec_memories(rowid, embedding)
        VALUES (?, ?)
      SQL
    end

    def vector_search(query_embedding, limit: 10, created_after: nil, created_before: nil)
      return [] unless @vectors_enabled

      query_blob = query_embedding.pack("e*")

      sql = +<<~SQL
        SELECT
          m.*,
          v.distance
        FROM vec_memories v
        JOIN memories m ON m.id = v.rowid
        WHERE v.embedding MATCH ?
          AND k = ?
      SQL
      params = [query_blob, limit]

      append_date_filters(sql, params, created_after, created_before, column: "m.created_at")

      sql += " ORDER BY v.distance"

      results = @db.execute(sql, params).map { |row| deserialize_with_distance(row) }
      # Filter out low-relevance results (distance > threshold means unrelated)
      results.select { |r| r["distance"] <= Recollect.config.max_vector_distance }
    end

    def embedding_count
      return 0 unless @vectors_enabled

      @db.get_first_value("SELECT COUNT(*) FROM vec_memories") || 0
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

    def next_day(date_string)
      date = Date.parse(date_string)
      (date + 1).to_s
    end

    def append_date_filters(sql, params, created_after, created_before, column: "created_at")
      return if created_after.nil? && created_before.nil?

      (sql << " AND #{column} >= ?" and params << created_after) if created_after
      (sql << " AND #{column} < ?" and params << next_day(created_before)) if created_before
    end

    def deserialize(row)
      return nil unless row

      result = {
        "id" => row["id"],
        "content" => row["content"],
        "memory_type" => row["memory_type"],
        "tags" => json_decode(row["tags"]),
        "metadata" => json_decode(row["metadata"]),
        "created_at" => row["created_at"],
        "updated_at" => row["updated_at"],
        "rank" => row["rank"]
      }.compact

      # Convert SQLite integer (0/1) to boolean if present
      result["has_embedding"] = row["has_embedding"] == 1 if row.key?("has_embedding")

      result
    end

    def deserialize_with_distance(row)
      result = deserialize(row)
      result["distance"] = row["distance"] if row["distance"]
      result
    end

    def json_decode(value)
      return nil unless value

      JSON.parse(value)
    rescue JSON::ParserError
      nil
    end
  end
end
