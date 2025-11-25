# frozen_string_literal: true

require "sqlite3"
require "json"

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
        created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        updated_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
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
        INSERT INTO memories_fts(memories_fts, rowid, content, tags, memory_type)
        VALUES('delete', old.id, old.content, old.tags, old.memory_type);
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

    def store(content:, memory_type: "note", tags: nil, metadata: nil, source: "unknown")
      raise ArgumentError, "content cannot be empty" if content.nil? || content.to_s.strip.empty?

      # Normalize tags to lowercase
      normalized_tags = tags&.map(&:downcase)

      @db.execute(<<~SQL, [content, memory_type, json_encode(normalized_tags), json_encode(metadata), source])
        INSERT INTO memories (content, memory_type, tags, metadata, source)
        VALUES (?, ?, ?, ?, ?)
      SQL
      @db.last_insert_row_id
    end

    def get(id)
      row = @db.get_first_row("SELECT * FROM memories WHERE id = ?", id)
      deserialize(row)
    end

    def search(query, memory_type: nil, limit: 10, offset: 0)
      # Escape query for FTS5 (treat as literal phrase)
      safe_query = "\"#{query.gsub('"', '""')}\""

      sql = <<~SQL
        SELECT memories.*, bm25(memories_fts) as rank
        FROM memories_fts
        JOIN memories ON memories.id = memories_fts.rowid
        WHERE memories_fts MATCH ?
      SQL
      params = [safe_query]

      if memory_type
        sql += " AND memories.memory_type = ?"
        params << memory_type
      end

      sql += " ORDER BY rank LIMIT ? OFFSET ?"
      params.push(limit, offset)

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def list(memory_type: nil, limit: 50, offset: 0)
      sql = "SELECT * FROM memories"
      params = []

      if memory_type
        sql += " WHERE memory_type = ?"
        params << memory_type
      end

      sql += " ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
      params.push(limit, offset)

      @db.execute(sql, params).map { |row| deserialize(row) }
    end

    def update(id, content: nil, tags: nil, metadata: nil) # rubocop:disable Naming/PredicateMethod
      updates = []
      params = []

      if content
        updates << "content = ?"
        params << content
      end

      if tags
        updates << "tags = ?"
        # Normalize tags to lowercase
        normalized_tags = tags.map(&:downcase)
        params << json_encode(normalized_tags)
      end

      if metadata
        updates << "metadata = ?"
        params << json_encode(metadata)
      end

      return false if updates.empty?

      updates << "updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
      params << id

      @db.execute("UPDATE memories SET #{updates.join(", ")} WHERE id = ?", params)
      @db.changes.positive?
    end

    def delete(id) # rubocop:disable Naming/PredicateMethod
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

    def search_by_tags(tag_filters, memory_type: nil, limit: 10)
      return [] if tag_filters.nil? || tag_filters.empty?

      # Normalize tag_filters to lowercase
      normalized_tags = tag_filters.map(&:downcase)

      sql = "SELECT * FROM memories WHERE 1=1"
      params = []

      # Build WHERE clause to match all tags (AND logic)
      normalized_tags.each do |tag|
        sql += " AND tags LIKE ?"
        params << "%\"#{tag}\"%"
      end

      if memory_type
        sql += " AND memory_type = ?"
        params << memory_type
      end

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
        "id" => row["id"],
        "content" => row["content"],
        "memory_type" => row["memory_type"],
        "tags" => json_decode(row["tags"]),
        "metadata" => json_decode(row["metadata"]),
        "created_at" => row["created_at"],
        "updated_at" => row["updated_at"],
        "source" => row["source"],
        "rank" => row["rank"]
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
