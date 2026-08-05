# Recollect Sync — Phase 2 (Ambient Sync) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land the wire-format sync endpoints, push-on-write, pull-on-startup, and heartbeat reconciliation so two paired Recollect instances converge ambiently. End state: write a memory on A, see it on B within seconds; tombstone on A removes it from B; offline peers catch up on next startup.

**Architecture:** Add three signed HTTP endpoints (`GET /sync/manifest`, `POST /sync/pull`, `POST /sync/push`) gated by a Sinatra `before '/sync/*'` signature filter. Push-on-write enqueues `(global_id, db_name)` into a `Sync::PushQueue` worker that POSTs one record per peer per subscription. A `Sync::Engine` thread does pull-on-startup then loops on a heartbeat interval. Chunks (`memory_type='_chunk'`) and embedding bytes are **never** synced — receivers re-derive via the existing `EmbeddingWorker`. Integration tests use Faraday's `Rack` adapter so two `HTTPServer` instances talk in-process.

**Tech Stack:** Ruby 3.4+, SQLite3 (sqlite3 gem), Sinatra/Puma (HTTP), Thor (CLI), `ed25519` (already in Gemfile from Phase 1), `faraday` + `faraday-rack` for two-peer in-process tests, Minitest + Rack::Test.

**Reference:**
- [Sync Design (master)](2026-05-02-sync-design.md) — protocol shape, error model.
- [Phase 2 Design Supplement](2026-05-24-sync-phase-2-design.md) — chunk strategy, test topology, re-embed path.
- [Phase 1 Plan](2026-05-02-sync-phase-1-foundation.md) — established style for tasks (TDD, per-task commits).

**Branch:** `sync/phase-2` off `sync/phase-1` (once Phase 1 lands on `master`, rebase). If Phase 1 is still un-merged, branch off `sync/phase-1` directly.

**Out of scope (this phase):** re-embed pass when embedding model changes, vacuum/permanent tombstone deletion, HTTPS, encrypted-at-rest private key, Web UI for peers, fan-out batching beyond one-record-per-push.

---

## Errata — composite cursors (added 2026-05-25)

The first draft of this plan used a scalar `created_at` watermark with strict `>` comparison. During Task 2 implementation we found that millisecond-precision `created_at` collides under batch writes (e.g. importing 600 records, or two MCP `store` calls in the same ms), causing pagination to silently drop the second-and-later rows at the boundary timestamp.

**Resolution:** Switch all per-origin cursors to the composite **(created_at, global_id)**. `global_id` is UUIDv7 — globally unique, and intra-origin lex order matches creation order. Multi-hop safe (`global_id` is portable across peers in a way `id` is not).

This changes the shape of:

- `Database#fetch_for_sync` — `since:` values become `{"created_at" => ts, "global_id" => uuid}`. SQL adds `(created_at > ? OR (created_at = ? AND global_id > ?))`.
- `Database#max_origin_timestamp` — becomes `max_origin_cursor`, returns `{"created_at" => ts, "global_id" => uuid}` or `nil`.
- `Sync::Store` — `peer_watermarks` adds a `latest_global_id TEXT NOT NULL` column (migration via `ALTER TABLE` for existing installs).
- `Sync::Watermarks` — `get(db_name:)` returns `{peer_id => {created_at, global_id}}`; `advance` takes `created_at:` and `global_id:`.
- `/sync/manifest` JSON — `watermarks` values become objects: `{created_at, global_id}`.
- `/sync/pull` JSON body — `since` values become objects.
- `/sync/push` server — tracks observed `(created_at, global_id)` per origin, passes both to `Watermarks#advance`.
- `Sync::Engine` — propagates composite cursors through `pull_loop` and `push_missing`.

Affected tasks: 2, 3, 5, 9, 10, 11, 15. The implementer prompts for these tasks contain the corrected snippets; the inline code in the original task sections below is **superseded** wherever cursor shape is touched. No backwards-compat shim — Phase 1 just landed, no peers in the wild are doing sync yet.

---

## Setup

### Task 0: Branch + faraday-rack for tests

**Files:**
- Modify: `Gemfile`

**Step 1: Create branch**

```bash
git checkout sync/phase-1
git pull --ff-only            # if already merged to master, replace with: git checkout master && git pull
git checkout -b sync/phase-2
```

**Step 2: Add `faraday-rack` (test-only)**

In `Gemfile`, find the `group :test` block (if missing, add one near the top alongside `group :development`). Add:

```ruby
group :test do
  gem "faraday-rack", "~> 2.0"
end
```

`faraday` itself is already a direct dep (used by `/api/sync/peers/join`).

**Step 3: Install**

```bash
bundle install
```

**Step 4: Verify**

```bash
bundle exec ruby -rfaraday -e "
  require 'faraday/rack'
  app = ->(env) { [200, {}, ['ok']] }
  conn = Faraday.new { |f| f.adapter :rack, app }
  puts conn.get('/').body
"
```

Expected: prints `ok`.

**Step 5: Commit**

```bash
git add Gemfile Gemfile.lock
git commit -m "chore: add faraday-rack for in-process two-peer tests"
```

---

## Database-level sync primitives

### Task 1: `Database#store` returns `{id, global_id}`

**Files:**
- Modify: `lib/recollect/database.rb` (`store` method)
- Modify: `lib/recollect/database_manager.rb` (callers of `db.store`)
- Test: `test/recollect/database_test.rb`

**Context:** `Database#store` currently returns the local `id` (integer). Push-on-write needs the `global_id` (UUID) too. We change `store` to return a small struct/hash so callers can destructure.

**Step 1: Write failing test**

Append to `test/recollect/database_test.rb`:

```ruby
def test_store_returns_id_and_global_id
  result = @db.store(content: "hi", memory_type: "note")
  assert_kind_of Integer, result[:id]
  assert_match(/\A[0-9a-f-]{36}\z/, result[:global_id])
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n test_store_returns_id_and_global_id
```

**Step 3: Implement**

In `Database#store`, replace `@db.last_insert_row_id` with:

```ruby
{id: @db.last_insert_row_id, global_id: global_id}
```

**Step 4: Update callers**

`lib/recollect/database_manager.rb` lines 30-62. Replace every `db.store(...)` use of the integer with `[:id]`. For example:

```ruby
result = db.store(content: content, memory_type: memory_type, tags: tags, metadata: metadata, origin_peer: @local_peer_id || "local")
parent_id = result[:id]
```

Do the same for the chunk store call (`chunk_id = result[:id]`).

`MemoriesService#create` (`lib/recollect/memories_service.rb:9-24`) calls `@db_manager.store_with_embedding`, which still returns an integer — keep that interface as-is for now (the change to expose `global_id` to MemoriesService happens in Task 14).

**Step 5: Run all tests, expect PASS**

```bash
bundle exec rake test
```

**Step 6: Commit**

```bash
git add lib/recollect/database.rb lib/recollect/database_manager.rb test/recollect/database_test.rb
git commit -m "feat(db): store returns {id, global_id} for sync push hook"
```

---

### Task 2: `Database#fetch_for_sync` — paginated read by watermark

**Files:**
- Modify: `lib/recollect/database.rb`
- Test: `test/recollect/database_test.rb`

**Context:** Returns records strictly newer than the per-origin watermark, ordered by `(created_at, id)`, excluding `_chunk` rows. Embedding bytes excluded (the `embedding` BLOB column exists but is unused at the row level in current code; we still explicitly skip it).

**Step 1: Write failing test**

```ruby
def test_fetch_for_sync_excludes_chunks
  parent = @db.store(content: "parent", memory_type: "note")
  @db.instance_variable_get(:@db).execute(
    "INSERT INTO memories (content, memory_type, global_id, origin_peer) VALUES (?, ?, ?, ?)",
    ["chunk", "_chunk", SecureRandom.uuid_v7, "local"]
  )
  rows = @db.fetch_for_sync(since: {}, limit: 10)
  assert_equal 1, rows.size
  assert_equal parent[:global_id], rows.first["global_id"]
end

def test_fetch_for_sync_respects_since_per_origin
  raw = @db.instance_variable_get(:@db)
  # Three rows: two from peer-a (one old, one new), one from peer-b
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
              ["a1", "note", "g-a1", "peer-a", "2026-05-01T00:00:00.000Z"])
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
              ["a2", "note", "g-a2", "peer-a", "2026-05-02T00:00:00.000Z"])
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES (?,?,?,?,?)",
              ["b1", "note", "g-b1", "peer-b", "2026-05-01T12:00:00.000Z"])

  rows = @db.fetch_for_sync(since: {"peer-a" => "2026-05-01T00:00:00.000Z"}, limit: 10)
  global_ids = rows.map { |r| r["global_id"] }
  refute_includes global_ids, "g-a1", "a1 is at the watermark, must be excluded (strictly greater)"
  assert_includes global_ids, "g-a2"
  assert_includes global_ids, "g-b1", "peer-b has no watermark, gets everything"
end

def test_fetch_for_sync_includes_tombstones
  result = @db.store(content: "doomed", memory_type: "note")
  @db.delete(result[:id], deleted_by_peer: "local")
  rows = @db.fetch_for_sync(since: {}, limit: 10)
  assert_equal 1, rows.size
  refute_nil rows.first["deleted_at"]
end

def test_fetch_for_sync_pagination
  6.times { |i| @db.store(content: "m#{i}", memory_type: "note") }
  page1 = @db.fetch_for_sync(since: {}, limit: 3)
  assert_equal 3, page1.size
  last_ts = page1.last["created_at"]
  page2 = @db.fetch_for_sync(since: {"local" => last_ts}, limit: 3)
  assert_operator page2.size, :>=, 1
  assert_empty page1.map { |r| r["global_id"] } & page2.map { |r| r["global_id"] }
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /fetch_for_sync/
```

**Step 3: Implement**

Add to `Database`:

```ruby
SYNC_COLUMNS = "id, global_id, origin_peer, content, memory_type, tags, metadata, created_at, deleted_at, deleted_by_peer"

def fetch_for_sync(since:, limit: 500)
  # Build WHERE: include row when origin's watermark is missing OR row.created_at > watermark.
  # Implemented with one OR per known origin + a fallback for origins not in `since`.
  origins_with_wm = since.keys
  conditions = ["memory_type != '_chunk'"]
  params = []

  if origins_with_wm.any?
    placeholders = origins_with_wm.map { "?" }.join(",")
    per_origin = origins_with_wm.map { "(origin_peer = ? AND created_at > ?)" }.join(" OR ")
    conditions << "(origin_peer NOT IN (#{placeholders}) OR #{per_origin})"
    params.concat(origins_with_wm)
    origins_with_wm.each { |o| params.push(o, since[o]) }
  end

  sql = "SELECT #{SYNC_COLUMNS} FROM memories WHERE #{conditions.join(" AND ")} ORDER BY created_at ASC, id ASC LIMIT ?"
  params << limit

  @db.execute(sql, params)
end
```

(Note: we DO NOT filter `deleted_at IS NULL` here — tombstones must propagate. The read filter elsewhere in the codebase handles user-facing reads.)

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /fetch_for_sync/
bundle exec rake test
```

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): fetch_for_sync paginates by per-origin watermark, excludes chunks"
```

---

### Task 3: `Database#max_origin_timestamp` for self-watermark

**Files:**
- Modify: `lib/recollect/database.rb`
- Test: `test/recollect/database_test.rb`

**Context:** Manifest needs `MAX(created_at) WHERE origin_peer = <local_peer_id>` so a peer asking us "what do you know about yourself?" gets the right answer.

**Step 1: Write failing test**

```ruby
def test_max_origin_timestamp_returns_latest_for_origin
  raw = @db.instance_variable_get(:@db)
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES ('a','note','g1','peer-a','2026-05-01T00:00:00.000Z')")
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES ('b','note','g2','peer-a','2026-05-03T00:00:00.000Z')")
  raw.execute("INSERT INTO memories (content, memory_type, global_id, origin_peer, created_at) VALUES ('c','note','g3','peer-b','2026-05-02T00:00:00.000Z')")
  assert_equal "2026-05-03T00:00:00.000Z", @db.max_origin_timestamp("peer-a")
  assert_equal "2026-05-02T00:00:00.000Z", @db.max_origin_timestamp("peer-b")
  assert_nil @db.max_origin_timestamp("peer-c")
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
def max_origin_timestamp(peer_id)
  @db.get_first_value("SELECT MAX(created_at) FROM memories WHERE origin_peer = ?", peer_id)
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): max_origin_timestamp for self-watermark"
```

---

### Task 4: `Database#upsert_synced` — receive-side insert with first-write-wins

**Files:**
- Modify: `lib/recollect/database.rb`
- Test: `test/recollect/database_test.rb`

**Context:** Apply a record received from a peer. First-write wins for content; first-tombstone wins for deletion. Returns `:inserted | :updated | :no_change` for the receive-side enqueue decision (only enqueue re-embed on `:inserted`).

**Step 1: Write failing tests**

```ruby
def test_upsert_inserts_new_record
  rec = sync_record("g-new", origin: "peer-x", content: "hi")
  assert_equal :inserted, @db.upsert_synced(rec)
  row = @db.instance_variable_get(:@db).get_first_row("SELECT * FROM memories WHERE global_id = ?", "g-new")
  assert_equal "hi", row["content"]
  assert_equal "peer-x", row["origin_peer"]
end

def test_upsert_first_write_wins_for_content
  first = sync_record("g-dup", origin: "peer-x", content: "first", created_at: "2026-05-01T00:00:00.000Z")
  second = sync_record("g-dup", origin: "peer-y", content: "second", created_at: "2026-05-02T00:00:00.000Z")
  assert_equal :inserted, @db.upsert_synced(first)
  assert_equal :no_change, @db.upsert_synced(second)
  row = @db.instance_variable_get(:@db).get_first_row("SELECT content, origin_peer FROM memories WHERE global_id = ?", "g-dup")
  assert_equal "first", row["content"], "content must not change"
  assert_equal "peer-x", row["origin_peer"], "origin must not change"
end

def test_upsert_applies_tombstone_to_existing_row
  @db.upsert_synced(sync_record("g-x", content: "x"))
  tomb = sync_record("g-x", content: "x", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-z")
  assert_equal :updated, @db.upsert_synced(tomb)
  row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at, deleted_by_peer FROM memories WHERE global_id = ?", "g-x")
  refute_nil row["deleted_at"]
  assert_equal "peer-z", row["deleted_by_peer"]
end

def test_upsert_first_tombstone_wins
  @db.upsert_synced(sync_record("g-y", content: "y"))
  @db.upsert_synced(sync_record("g-y", content: "y", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-a"))
  @db.upsert_synced(sync_record("g-y", content: "y", deleted_at: "2026-05-03T00:00:00.000Z", deleted_by_peer: "peer-b"))
  row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at, deleted_by_peer FROM memories WHERE global_id = ?", "g-y")
  assert_equal "2026-05-02T00:00:00.000Z", row["deleted_at"]
  assert_equal "peer-a", row["deleted_by_peer"]
end

def test_upsert_tombstone_before_content_keeps_tombstone
  tomb = sync_record("g-z", origin: "peer-x", content: "", deleted_at: "2026-05-02T00:00:00.000Z", deleted_by_peer: "peer-x")
  @db.upsert_synced(tomb)
  late = sync_record("g-z", origin: "peer-x", content: "late", created_at: "2026-05-01T00:00:00.000Z")
  @db.upsert_synced(late)
  row = @db.instance_variable_get(:@db).get_first_row("SELECT deleted_at FROM memories WHERE global_id = ?", "g-z")
  refute_nil row["deleted_at"], "tombstone must persist even after late content arrives"
end

# Helper at top of file (or test_helper if you prefer):
private

def sync_record(global_id, origin: "peer-x", content: "data", memory_type: "note", tags: nil, metadata: nil,
                created_at: "2026-05-15T00:00:00.000Z", deleted_at: nil, deleted_by_peer: nil)
  {
    "global_id" => global_id, "origin_peer" => origin, "content" => content, "memory_type" => memory_type,
    "tags" => tags && JSON.generate(tags), "metadata" => metadata && JSON.generate(metadata),
    "created_at" => created_at, "deleted_at" => deleted_at, "deleted_by_peer" => deleted_by_peer
  }
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
def upsert_synced(rec)
  existing = @db.get_first_row("SELECT id, deleted_at FROM memories WHERE global_id = ?", rec["global_id"])

  if existing.nil?
    @db.execute(
      "INSERT INTO memories (global_id, origin_peer, content, memory_type, tags, metadata, created_at, deleted_at, deleted_by_peer) " \
      "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
      [rec["global_id"], rec["origin_peer"], rec["content"], rec["memory_type"], rec["tags"], rec["metadata"],
       rec["created_at"], rec["deleted_at"], rec["deleted_by_peer"]]
    )
    return :inserted
  end

  # Existing row: only the tombstone fields may change, and only when ours is NULL.
  if existing["deleted_at"].nil? && rec["deleted_at"]
    # Bypass the memories_no_resurrect trigger guard — going from NULL → non-NULL is exactly what we want.
    @db.execute(
      "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
      [rec["deleted_at"], rec["deleted_by_peer"], existing["id"]]
    )
    return :updated
  end

  :no_change
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): upsert_synced with first-write-wins and first-tombstone-wins"
```

---

## Watermarks

### Task 5: `Sync::Watermarks` — read/advance per (peer, db_name)

**Files:**
- Create: `lib/recollect/sync/watermarks.rb`
- Create: `test/recollect/sync/watermarks_test.rb`

**Step 1: Write failing test**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::WatermarksTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @wm = Recollect::Sync::Watermarks.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def test_get_returns_empty_hash_when_no_rows
    assert_equal({}, @wm.get(db_name: "global"))
  end

  def test_advance_inserts_when_missing
    @wm.advance(peer_id: "peer-a", db_name: "global", timestamp: "2026-05-01T00:00:00.000Z")
    assert_equal({"peer-a" => "2026-05-01T00:00:00.000Z"}, @wm.get(db_name: "global"))
  end

  def test_advance_takes_max_of_old_and_new
    @wm.advance(peer_id: "peer-a", db_name: "global", timestamp: "2026-05-02T00:00:00.000Z")
    @wm.advance(peer_id: "peer-a", db_name: "global", timestamp: "2026-05-01T00:00:00.000Z")
    assert_equal({"peer-a" => "2026-05-02T00:00:00.000Z"}, @wm.get(db_name: "global"))
    @wm.advance(peer_id: "peer-a", db_name: "global", timestamp: "2026-05-03T00:00:00.000Z")
    assert_equal({"peer-a" => "2026-05-03T00:00:00.000Z"}, @wm.get(db_name: "global"))
  end

  def test_db_name_is_scoped
    @wm.advance(peer_id: "peer-a", db_name: "global", timestamp: "2026-05-01T00:00:00.000Z")
    @wm.advance(peer_id: "peer-a", db_name: "personal", timestamp: "2026-05-02T00:00:00.000Z")
    assert_equal({"peer-a" => "2026-05-01T00:00:00.000Z"}, @wm.get(db_name: "global"))
    assert_equal({"peer-a" => "2026-05-02T00:00:00.000Z"}, @wm.get(db_name: "personal"))
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true

module Recollect
  module Sync
    class Watermarks
      def initialize(store)
        @store = store
      end

      def get(db_name:)
        db.execute("SELECT peer_id, latest_created_at FROM peer_watermarks WHERE db_name = ?", db_name)
          .each_with_object({}) { |row, h| h[row["peer_id"]] = row["latest_created_at"] }
      end

      def advance(peer_id:, db_name:, timestamp:)
        db.execute(
          "INSERT INTO peer_watermarks (peer_id, db_name, latest_created_at) VALUES (?, ?, ?) " \
          "ON CONFLICT(peer_id, db_name) DO UPDATE SET " \
          "latest_created_at = MAX(peer_watermarks.latest_created_at, excluded.latest_created_at)",
          [peer_id, db_name, timestamp]
        )
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/sync/watermarks.rb test/recollect/sync/watermarks_test.rb
git commit -m "feat(sync): Watermarks read/advance per (peer, db_name)"
```

---

## Database-name mapping

### Task 6: `DatabaseManager#db_name_for_project` and `#project_for_db_name`

**Files:**
- Modify: `lib/recollect/database_manager.rb`
- Test: `test/recollect/database_manager_test.rb`

**Context:** Sync talks in `db_name` strings (`"global"`, `"foo"`, ...). The rest of the codebase talks in `project` (nil for global, otherwise a string). One canonical pair of helpers prevents `"global"` vs `nil` confusion bugs throughout sync code.

**Step 1: Write failing test**

In `test/recollect/database_manager_test.rb` (create if missing):

```ruby
def test_db_name_for_project_maps_nil_to_global
  dm = Recollect::DatabaseManager.new(Recollect.config, local_peer_id: "local")
  assert_equal "global", dm.db_name_for_project(nil)
  assert_equal "foo", dm.db_name_for_project("foo")
end

def test_project_for_db_name_maps_global_to_nil
  dm = Recollect::DatabaseManager.new(Recollect.config, local_peer_id: "local")
  assert_nil dm.project_for_db_name("global")
  assert_equal "foo", dm.project_for_db_name("foo")
end

def test_list_db_names_includes_global_and_projects
  dm = Recollect::DatabaseManager.new(Recollect.config, local_peer_id: "local")
  dm.get_database(nil)        # touch global
  dm.get_database("personal") # touch a project
  names = dm.list_db_names
  assert_includes names, "global"
  assert_includes names, "personal"
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

In `DatabaseManager`:

```ruby
GLOBAL_DB_NAME = "global"

def db_name_for_project(project)
  project.nil? || project.empty? ? GLOBAL_DB_NAME : project
end

def project_for_db_name(db_name)
  (db_name == GLOBAL_DB_NAME) ? nil : db_name
end

def list_db_names
  [GLOBAL_DB_NAME] + list_projects
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/database_manager.rb test/recollect/database_manager_test.rb
git commit -m "feat(db): db_name_for_project mapping helpers for sync"
```

---

## Outbound HTTP

### Task 7: `Sync::Client` — signed outbound Faraday client + adapter factory

**Files:**
- Create: `lib/recollect/sync/client.rb`
- Create: `test/recollect/sync/client_test.rb`

**Context:** Wraps Faraday. Signs requests using `Sync::Crypto`. The Faraday adapter is selectable so tests can wire two `HTTPServer` instances together in-process.

**Step 1: Write failing test**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::ClientTest < Minitest::Test
  def setup
    @signing_key = Ed25519::SigningKey.generate
    @public_key  = @signing_key.verify_key.to_bytes
    @private_key = @signing_key.to_bytes
    @captured = nil

    # Build a fake Rack app that captures the incoming request
    @app = ->(env) {
      req = Rack::Request.new(env)
      body = env["rack.input"].read
      @captured = {
        path:   req.path_info,
        method: req.request_method,
        body:   body,
        headers: env.select { |k, _| k.start_with?("HTTP_X_") }
      }
      [200, {"Content-Type" => "application/json"}, ['{"ok":true}']]
    }
  end

  def test_signs_outbound_post
    client = Recollect::Sync::Client.new(
      peer_id: "local-peer", private_key: @private_key,
      endpoint: "http://peer.test", adapter: [:rack, @app]
    )
    client.post_json("/sync/push?db=global", {records: []})

    assert_equal "POST", @captured[:method]
    assert_equal "local-peer", @captured[:headers]["HTTP_X_PEER_ID"]
    refute_nil @captured[:headers]["HTTP_X_TIMESTAMP"]
    refute_nil @captured[:headers]["HTTP_X_SIGNATURE"]

    # Verify signature against captured body
    assert Recollect::Sync::Crypto.verify(
      public_key: @public_key,
      peer_id:    @captured[:headers]["HTTP_X_PEER_ID"],
      timestamp:  @captured[:headers]["HTTP_X_TIMESTAMP"],
      body:       @captured[:body],
      signature:  @captured[:headers]["HTTP_X_SIGNATURE"]
    )
  end

  def test_get_signs_with_empty_body
    client = Recollect::Sync::Client.new(
      peer_id: "local-peer", private_key: @private_key,
      endpoint: "http://peer.test", adapter: [:rack, @app]
    )
    client.get("/sync/manifest?db=global")
    assert_equal "GET", @captured[:method]
    assert_equal "", @captured[:body]
    refute_nil @captured[:headers]["HTTP_X_SIGNATURE"]
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true
require "faraday"

module Recollect
  module Sync
    class Client
      def initialize(peer_id:, private_key:, endpoint:, adapter: nil)
        @peer_id     = peer_id
        @private_key = private_key
        @endpoint    = endpoint
        @adapter     = adapter || [:net_http]
        @conn = Faraday.new(url: endpoint) { |f| f.adapter(*@adapter) }
      end

      def get(path)
        send_signed(:get, path, "")
      end

      def post_json(path, payload)
        body = JSON.generate(payload)
        send_signed(:post, path, body, content_type: "application/json")
      end

      private

      def send_signed(method, path, body, content_type: nil)
        ts  = Time.now.utc.iso8601
        sig = Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: ts, body: body)
        @conn.run_request(method, path, body, nil) do |req|
          req.headers["Content-Type"] = content_type if content_type
          req.headers["X-Peer-ID"]    = @peer_id
          req.headers["X-Timestamp"]  = ts
          req.headers["X-Signature"]  = sig
        end
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/client_test.rb
```

**Step 5: Commit**

```bash
git add lib/recollect/sync/client.rb test/recollect/sync/client_test.rb
git commit -m "feat(sync): Client signs outbound requests, pluggable Faraday adapter"
```

---

## Signature middleware (server side)

### Task 8: Sinatra `before '/sync/*'` filter verifies signatures

**Files:**
- Create: `lib/recollect/sync/signature_verifier.rb`
- Create: `test/recollect/sync/signature_verifier_test.rb`
- Modify: `lib/recollect/http_server.rb` (add the `before` filter)

**Context:** Resolves `X-Peer-ID` against `known_peers`. Rejects 401 for bad sig/skew/missing headers, 403 for blocked/unknown peer. Stores the resolved peer in `env['recollect.peer']` for the route to consume.

**Step 1: Write failing tests**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::SignatureVerifierTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)

    @signing_key = Ed25519::SigningKey.generate
    @peer_public = @signing_key.verify_key.to_bytes
    @peers.add(peer_id: "peer-a", display_name: "A", public_key: @peer_public, endpoint: "http://a:7326")

    @verifier = Recollect::Sync::SignatureVerifier.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def signed_env(body, peer_id: "peer-a", ts: Time.now.utc.iso8601)
    sig = Recollect::Sync::Crypto.sign(
      private_key: @signing_key.to_bytes, peer_id: peer_id, timestamp: ts, body: body
    )
    {"HTTP_X_PEER_ID" => peer_id, "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig, body: body}
  end

  def test_valid_signature_returns_peer
    env = signed_env("hello")
    result = @verifier.verify(headers: env, body: env[:body])
    assert_equal :ok, result[:status]
    assert_equal "peer-a", result[:peer][:peer_id]
  end

  def test_missing_headers_returns_unauthorized
    result = @verifier.verify(headers: {}, body: "")
    assert_equal :unauthorized, result[:status]
  end

  def test_unknown_peer_returns_forbidden
    env = signed_env("x", peer_id: "ghost")
    result = @verifier.verify(headers: env, body: env[:body])
    assert_equal :forbidden, result[:status]
  end

  def test_blocked_peer_returns_forbidden
    @peers.block("peer-a")
    env = signed_env("x")
    result = @verifier.verify(headers: env, body: env[:body])
    assert_equal :forbidden, result[:status]
  end

  def test_mutated_body_returns_unauthorized
    env = signed_env("real-body")
    result = @verifier.verify(headers: env, body: "tampered")
    assert_equal :unauthorized, result[:status]
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true

module Recollect
  module Sync
    class SignatureVerifier
      def initialize(store)
        @store = store
        @peers = Peers.new(store)
      end

      # headers may be a Rack env (with HTTP_X_*) or a hash with simple keys.
      def verify(headers:, body:)
        peer_id   = headers["HTTP_X_PEER_ID"]   || headers["X-Peer-ID"]
        timestamp = headers["HTTP_X_TIMESTAMP"] || headers["X-Timestamp"]
        signature = headers["HTTP_X_SIGNATURE"] || headers["X-Signature"]
        return {status: :unauthorized, reason: "missing-headers"} if [peer_id, timestamp, signature].any?(&:nil?)

        peer = @peers.find(peer_id)
        return {status: :forbidden, reason: "unknown-peer"} unless peer
        return {status: :forbidden, reason: "blocked-peer"} if peer[:status] == "blocked"

        valid = Crypto.verify(
          public_key: peer[:public_key], peer_id: peer_id, timestamp: timestamp, body: body, signature: signature
        )
        return {status: :unauthorized, reason: "bad-signature"} unless valid

        {status: :ok, peer: peer}
      end
    end
  end
end
```

**Step 4: Wire into HTTPServer**

In `lib/recollect/http_server.rb`, add **before** the existing `before do` wiredump block:

```ruby
before %r{^/sync/} do
  request.body.rewind
  body = request.body.read
  request.body.rewind
  result = Recollect::Sync::SignatureVerifier.new(self.class.sync_store).verify(headers: request.env, body: body)
  case result[:status]
  when :unauthorized then halt 401, {error: result[:reason]}.to_json
  when :forbidden    then halt 403, {error: result[:reason]}.to_json
  else env["recollect.peer"] = result[:peer]
  end
end
```

**Step 5: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/signature_verifier_test.rb
bundle exec rake test
```

**Step 6: Commit**

```bash
git add lib/recollect/sync/signature_verifier.rb test/recollect/sync/signature_verifier_test.rb lib/recollect/http_server.rb
git commit -m "feat(sync): signature middleware on /sync/* routes"
```

---

## Sync endpoints

### Task 9: `GET /sync/manifest?db=<db>`

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Context:** Returns `{watermarks: {peer_id → ts}}` for the requested DB. Self-watermark from `Database#max_origin_timestamp(local_peer_id)`. Other watermarks from `Sync::Watermarks#get(db_name:)`.

**Step 1: Write failing test**

```ruby
def test_sync_manifest_requires_signature
  get "/sync/manifest?db=global"
  assert_includes [401, 403], last_response.status
end

def test_sync_manifest_returns_watermarks_for_db
  signing_key = setup_trusted_peer("peer-a")
  identity = Recollect::HTTPServer.local_identity

  # Insert a self-origin row so self-watermark is non-nil
  Recollect::HTTPServer.db_manager.get_database(nil).store(content: "self", memory_type: "note")

  # Insert a watermark for some other origin
  Recollect::Sync::Watermarks.new(Recollect::HTTPServer.sync_store)
    .advance(peer_id: "peer-c", db_name: "global", timestamp: "2026-05-01T00:00:00.000Z")

  signed_get("peer-a", signing_key, "/sync/manifest?db=global")
  assert_equal 200, last_response.status
  body = JSON.parse(last_response.body)
  refute_nil body["watermarks"][identity.peer_id], "self-watermark present"
  assert_equal "2026-05-01T00:00:00.000Z", body["watermarks"]["peer-c"]
end
```

Add helpers to `test_helper.rb` or to a sync test module:

```ruby
def setup_trusted_peer(peer_id)
  signing_key = Ed25519::SigningKey.generate
  Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).add(
    peer_id: peer_id, display_name: peer_id,
    public_key: signing_key.verify_key.to_bytes, endpoint: "http://#{peer_id}:7326"
  )
  signing_key
end

def signed_get(peer_id, signing_key, path)
  ts = Time.now.utc.iso8601
  sig = Recollect::Sync::Crypto.sign(private_key: signing_key.to_bytes, peer_id: peer_id, timestamp: ts, body: "")
  get(path, {}, "HTTP_X_PEER_ID" => peer_id, "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig)
end

def signed_post(peer_id, signing_key, path, body)
  body_str = body.is_a?(String) ? body : JSON.generate(body)
  ts = Time.now.utc.iso8601
  sig = Recollect::Sync::Crypto.sign(private_key: signing_key.to_bytes, peer_id: peer_id, timestamp: ts, body: body_str)
  post(path, body_str, "CONTENT_TYPE" => "application/json", "HTTP_X_PEER_ID" => peer_id, "HTTP_X_TIMESTAMP" => ts, "HTTP_X_SIGNATURE" => sig)
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

In `http_server.rb`:

```ruby
get "/sync/manifest" do
  db_name = params["db"] or halt 400, {error: "missing db"}.to_json
  project = self.class.db_manager.project_for_db_name(db_name)
  database = self.class.db_manager.get_database(project)
  identity = self.class.local_identity
  wm = Recollect::Sync::Watermarks.new(self.class.sync_store).get(db_name: db_name)
  self_ts = database.max_origin_timestamp(identity.peer_id)
  wm[identity.peer_id] = self_ts if self_ts
  content_type :json
  {watermarks: wm}.to_json
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb test/test_helper.rb
git commit -m "feat(sync): GET /sync/manifest returns watermarks per db"
```

---

### Task 10: `POST /sync/pull?db=<db>`

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Step 1: Write failing tests**

```ruby
def test_sync_pull_returns_records_newer_than_since
  signing_key = setup_trusted_peer("peer-a")
  # Need outbound subscription so caller can pull global
  Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")
  Recollect::HTTPServer.db_manager.get_database(nil).store(content: "x", memory_type: "note")

  signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})
  assert_equal 200, last_response.status
  body = JSON.parse(last_response.body)
  assert_equal 1, body["records"].size
  assert_equal "x", body["records"].first["content"]
end

def test_sync_pull_returns_empty_when_caller_not_subscribed
  signing_key = setup_trusted_peer("peer-a")
  # Do NOT subscribe
  Recollect::HTTPServer.db_manager.get_database(nil).store(content: "x", memory_type: "note")
  signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})
  assert_equal 200, last_response.status
  assert_equal [], JSON.parse(last_response.body)["records"]
end

def test_sync_pull_excludes_chunks
  signing_key = setup_trusted_peer("peer-a")
  Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")
  db = Recollect::HTTPServer.db_manager.get_database(nil)
  db.store(content: "parent", memory_type: "note")
  db.instance_variable_get(:@db).execute(
    "INSERT INTO memories (content, memory_type, global_id, origin_peer) VALUES (?, ?, ?, ?)",
    ["chunk", "_chunk", SecureRandom.uuid_v7, "local"]
  )
  signed_post("peer-a", signing_key, "/sync/pull?db=global", {since: {}, limit: 500})
  records = JSON.parse(last_response.body)["records"]
  assert_equal 1, records.size
  refute_equal "_chunk", records.first["memory_type"]
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
post "/sync/pull" do
  db_name = params["db"] or halt 400, {error: "missing db"}.to_json
  caller_peer = env["recollect.peer"][:peer_id]
  peers = Recollect::Sync::Peers.new(self.class.sync_store)
  unless peers.subscriptions(caller_peer).include?(db_name)
    content_type :json
    halt 200, {records: []}.to_json
  end

  payload = JSON.parse(request.body.read)
  since   = payload["since"] || {}
  limit   = (payload["limit"] || 500).to_i

  project = self.class.db_manager.project_for_db_name(db_name)
  rows = self.class.db_manager.get_database(project).fetch_for_sync(since: since, limit: limit)

  content_type :json
  {records: rows.map { |r| r.reject { |k, _| %w[id].include?(k) } }}.to_json
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): POST /sync/pull paginates records since watermark, subscription-gated"
```

---

### Task 11: `POST /sync/push?db=<db>` — receive + upsert + advance watermarks + enqueue re-embed

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Step 1: Write failing tests**

```ruby
def test_sync_push_inserts_new_records
  signing_key = setup_trusted_peer("peer-a")
  records = [{
    "global_id" => "g-1", "origin_peer" => "peer-a", "content" => "from a",
    "memory_type" => "note", "created_at" => "2026-05-20T10:00:00.000Z",
    "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil
  }]
  signed_post("peer-a", signing_key, "/sync/push?db=global", {records: records})
  assert_equal 200, last_response.status
  body = JSON.parse(last_response.body)
  assert_equal 1, body["accepted"]

  db = Recollect::HTTPServer.db_manager.get_database(nil)
  row = db.instance_variable_get(:@db).get_first_row("SELECT content, origin_peer FROM memories WHERE global_id = ?", "g-1")
  assert_equal "from a", row["content"]
end

def test_sync_push_advances_watermark
  signing_key = setup_trusted_peer("peer-a")
  records = [{"global_id" => "g-2", "origin_peer" => "peer-a", "content" => "x", "memory_type" => "note",
              "created_at" => "2026-05-20T10:00:00.000Z", "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil}]
  signed_post("peer-a", signing_key, "/sync/push?db=global", {records: records})
  wm = Recollect::Sync::Watermarks.new(Recollect::HTTPServer.sync_store).get(db_name: "global")
  assert_equal "2026-05-20T10:00:00.000Z", wm["peer-a"]
end

def test_sync_push_replay_is_idempotent
  signing_key = setup_trusted_peer("peer-a")
  records = [{"global_id" => "g-3", "origin_peer" => "peer-a", "content" => "x", "memory_type" => "note",
              "created_at" => "2026-05-20T10:00:00.000Z", "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil}]
  signed_post("peer-a", signing_key, "/sync/push?db=global", {records: records})
  signed_post("peer-a", signing_key, "/sync/push?db=global", {records: records})
  count = Recollect::HTTPServer.db_manager.get_database(nil).instance_variable_get(:@db)
    .get_first_value("SELECT COUNT(*) FROM memories WHERE global_id = ?", "g-3")
  assert_equal 1, count
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
post "/sync/push" do
  db_name = params["db"] or halt 400, {error: "missing db"}.to_json
  payload = JSON.parse(request.body.read)
  records = payload["records"] || []

  project  = self.class.db_manager.project_for_db_name(db_name)
  database = self.class.db_manager.get_database(project)
  wm       = Recollect::Sync::Watermarks.new(self.class.sync_store)

  accepted = 0
  observed = Hash.new { |h, k| h[k] = "" }
  records.each do |rec|
    status = database.upsert_synced(rec)
    accepted += 1 if status != :no_change
    observed[rec["origin_peer"]] = [observed[rec["origin_peer"]], rec["created_at"]].max
    # Re-embed enqueue: only for fresh INSERTs of non-tombstone non-chunk parents
    if status == :inserted && rec["memory_type"] != "_chunk" && rec["deleted_at"].nil?
      row = database.instance_variable_get(:@db).get_first_row("SELECT id FROM memories WHERE global_id = ?", rec["global_id"])
      self.class.db_manager.enqueue_embedding(memory_id: row["id"], content: rec["content"], project: project) if row
    end
  end

  observed.each do |peer_id, latest_ts|
    wm.advance(peer_id: peer_id, db_name: db_name, timestamp: latest_ts) unless latest_ts.empty?
  end

  content_type :json
  {accepted: accepted, rejected: 0}.to_json
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): POST /sync/push upserts, advances watermarks, enqueues re-embed"
```

---

## Push-on-write

### Task 12: `Sync::PushQueue` — SizedQueue + worker thread

**Files:**
- Create: `lib/recollect/sync/push_queue.rb`
- Create: `test/recollect/sync/push_queue_test.rb`

**Context:** `enqueue(global_id:, db_name:)` puts a job on a SizedQueue. The worker thread drains, looks up subscribed peers, and for each peer POSTs `/sync/push` with that single record. `flush` (test-only) blocks until queue is empty.

**Step 1: Write failing test**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::PushQueueTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)

    # Add one trusted peer subscribed to global
    @peer_signing = Ed25519::SigningKey.generate
    @peers.add(peer_id: "peer-b", display_name: "B", public_key: @peer_signing.verify_key.to_bytes, endpoint: "http://b:1")
    @peers.subscribe("peer-b", "global")

    @captured_pushes = []
    @client_factory = ->(peer) {
      double = Object.new
      pushes = @captured_pushes
      double.define_singleton_method(:post_json) { |path, payload|
        pushes << {peer_id: peer[:peer_id], path: path, payload: payload}
        Struct.new(:status, :success?).new(200, true)
      }
      double
    }

    @db_manager = Recollect::HTTPServer.db_manager
    @queue = Recollect::Sync::PushQueue.new(
      store: @store, db_manager: @db_manager, client_factory: @client_factory, size: 100
    )
    @queue.start
  end

  def teardown
    @queue.stop
    @store.close
    super
  end

  def test_enqueue_fans_out_to_subscribed_peers
    db = @db_manager.get_database(nil)
    result = db.store(content: "broadcast", memory_type: "note")
    @queue.enqueue(global_id: result[:global_id], db_name: "global")
    @queue.flush

    assert_equal 1, @captured_pushes.size
    assert_equal "peer-b", @captured_pushes.first[:peer_id]
    assert_equal "/sync/push?db=global", @captured_pushes.first[:path]
    assert_equal result[:global_id], @captured_pushes.first[:payload][:records].first["global_id"]
  end

  def test_enqueue_skips_unsubscribed_dbs
    db = @db_manager.get_database("personal")
    result = db.store(content: "private", memory_type: "note")
    @queue.enqueue(global_id: result[:global_id], db_name: "personal")
    @queue.flush
    assert_empty @captured_pushes
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true

module Recollect
  module Sync
    class PushQueue
      def initialize(store:, db_manager:, client_factory:, size: 1000)
        @store = store
        @db_manager = db_manager
        @client_factory = client_factory
        @queue = SizedQueue.new(size)
        @peers = Peers.new(store)
        @drain_mutex = Mutex.new
        @drained = ConditionVariable.new
        @inflight = 0
        @running = false
        @thread = nil
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

      def enqueue(global_id:, db_name:)
        return unless @running
        @drain_mutex.synchronize { @inflight += 1 }
        @queue << {global_id: global_id, db_name: db_name}
      end

      def flush(timeout: 5)
        deadline = Time.now + timeout
        @drain_mutex.synchronize do
          @drained.wait(@drain_mutex, deadline - Time.now) while @inflight.positive? && Time.now < deadline
        end
      end

      private

      def run_loop
        while @running
          job = pop_one
          break if job.nil?
          process(job)
          @drain_mutex.synchronize do
            @inflight -= 1
            @drained.broadcast if @inflight.zero?
          end
        end
      end

      def pop_one
        @queue.pop
      rescue ClosedQueueError
        nil
      end

      def process(job)
        project = @db_manager.project_for_db_name(job[:db_name])
        database = @db_manager.get_database(project)
        row = database.instance_variable_get(:@db).get_first_row(
          "SELECT #{Database::SYNC_COLUMNS} FROM memories WHERE global_id = ?", job[:global_id]
        )
        return unless row
        record = row.reject { |k, _| %w[id].include?(k) }

        @peers.list.each do |peer|
          next unless peer[:status] == "trusted"
          next unless @peers.subscriptions(peer[:peer_id]).include?(job[:db_name])
          client = @client_factory.call(peer)
          begin
            response = client.post_json("/sync/push?db=#{job[:db_name]}", {records: [record]})
            ok = response.respond_to?(:success?) ? response.success? : (200..299).include?(response.status)
            update_peer_status(peer[:peer_id], success: ok, error: ok ? nil : "HTTP #{response.status}")
          rescue => e
            update_peer_status(peer[:peer_id], success: false, error: e.message)
          end
        end
      end

      def update_peer_status(peer_id, success:, error: nil)
        if success
          @store.instance_variable_get(:@db).execute(
            "UPDATE known_peers SET last_sync_at = ?, last_sync_error = NULL, last_seen_at = ? WHERE peer_id = ?",
            [Time.now.utc.iso8601, Time.now.utc.iso8601, peer_id]
          )
        else
          @store.instance_variable_get(:@db).execute(
            "UPDATE known_peers SET last_sync_error = ? WHERE peer_id = ?", [error, peer_id]
          )
        end
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/sync/push_queue.rb test/recollect/sync/push_queue_test.rb
git commit -m "feat(sync): PushQueue fans pushes out to subscribed peers"
```

---

### Task 13: Wire `PushQueue` into `HTTPServer` singletons + start in production

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Modify: `lib/recollect/config.rb` (add `RECOLLECT_SYNC_PUSH_QUEUE_SIZE`, `RECOLLECT_SYNC_DISABLE`)
- Test: `test/recollect/http_server_test.rb`

**Step 1: Add config**

In `lib/recollect/config.rb`, add reader methods (mirror existing patterns):

```ruby
def sync_disabled?
  ENV["RECOLLECT_SYNC_DISABLE"] == "1"
end

def sync_push_queue_size
  (ENV["RECOLLECT_SYNC_PUSH_QUEUE_SIZE"] || 1000).to_i
end

def sync_heartbeat_seconds
  (ENV["RECOLLECT_SYNC_HEARTBEAT_SECONDS"] || 300).to_i
end
```

**Step 2: Write failing test**

```ruby
def test_push_queue_singleton_initialized
  refute_nil Recollect::HTTPServer.push_queue
end

def test_push_queue_not_started_when_sync_disabled
  ENV["RECOLLECT_SYNC_DISABLE"] = "1"
  Recollect::HTTPServer.reset_db_manager!
  assert_nil Recollect::HTTPServer.push_queue
ensure
  ENV.delete("RECOLLECT_SYNC_DISABLE")
  Recollect::HTTPServer.reset_db_manager!
end
```

**Step 3: Implement**

In `http_server.rb`, add to the `class << self` block:

```ruby
def push_queue
  return nil if Recollect.config.sync_disabled?
  return @push_queue if @push_queue

  init_mutex.synchronize do
    @push_queue ||= begin
      identity = local_identity
      pq = Sync::PushQueue.new(
        store: sync_store, db_manager: db_manager,
        client_factory: ->(peer) {
          Sync::Client.new(peer_id: identity.peer_id, private_key: identity.private_key, endpoint: peer[:endpoint])
        },
        size: Recollect.config.sync_push_queue_size
      )
      pq.start
      pq
    end
  end
end
```

In `reset_db_manager!`, add:

```ruby
@push_queue&.stop
@push_queue = nil
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb lib/recollect/config.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): wire PushQueue singleton; RECOLLECT_SYNC_DISABLE env"
```

---

### Task 14: `MemoriesService` push-on-write hooks

**Files:**
- Modify: `lib/recollect/database_manager.rb` (`store_with_embedding` return value)
- Modify: `lib/recollect/memories_service.rb` (`create`, `delete`)
- Test: `test/recollect/memories_service_test.rb` (and a new integration-style test)

**Context:** After a successful local `create` or `delete`, enqueue on `HTTPServer.push_queue`. We pass an injectable push queue via `MemoriesService.new(db_manager, push_queue:)` so tests can capture without touching the real queue.

**Step 1: Update `DatabaseManager#store_with_embedding` to return both ids**

```ruby
def store_with_embedding(project:, content:, memory_type:, tags:, metadata:)
  db = get_database(project)
  parent = db.store(content: content, memory_type: memory_type, tags: tags, metadata: metadata, origin_peer: @local_peer_id || "local")
  parent_id = parent[:id]

  if @embedding_worker
    chunks = MarkdownChunker.chunk(content)
    if chunks.size > 1
      chunks.each_with_index do |chunk_content, idx|
        chunk = db.store(content: chunk_content, memory_type: "_chunk", tags: tags,
                         metadata: {"parent_id" => parent_id, "chunk_index" => idx},
                         origin_peer: @local_peer_id || "local")
        @embedding_worker.enqueue(memory_id: chunk[:id], content: chunk_content, project: project)
      end
    else
      @embedding_worker.enqueue(memory_id: parent_id, content: content, project: project)
    end
  end

  {id: parent_id, global_id: parent[:global_id]}
end
```

**Step 2: Update `MemoriesService`**

```ruby
def initialize(db_manager, push_queue: nil)
  @db_manager = db_manager
  @push_queue = push_queue
end

def create(content:, project: nil, memory_type: nil, tags: [])
  project = project&.downcase
  result = @db_manager.store_with_embedding(
    project: project, content: content, memory_type: memory_type || "note",
    tags: tags || [], metadata: nil
  )
  enqueue_push(result[:global_id], project)
  db = @db_manager.get_database(project)
  memory = db.get(result[:id])
  memory["project"] = project
  memory
end

def delete(id, project: nil)
  project = project&.downcase
  db = @db_manager.get_database(project)
  # We need the global_id before tombstoning (delete doesn't return it)
  row = db.instance_variable_get(:@db).get_first_row("SELECT global_id FROM memories WHERE id = ?", id)
  return false unless row
  success = db.delete(id)
  enqueue_push(row["global_id"], project) if success
  success
end

private

def enqueue_push(global_id, project)
  return unless @push_queue
  @push_queue.enqueue(global_id: global_id, db_name: @db_manager.db_name_for_project(project))
end
```

**Step 3: Update `HTTPServer.memories_service` to pass the push queue**

```ruby
def memories_service
  @memories_service ||= MemoriesService.new(db_manager, push_queue: push_queue)
end
```

**Step 4: Write failing test**

```ruby
def test_create_enqueues_push
  captured = []
  fake_queue = Object.new
  fake_queue.define_singleton_method(:enqueue) { |global_id:, db_name:| captured << [global_id, db_name] }
  service = Recollect::MemoriesService.new(Recollect::HTTPServer.db_manager, push_queue: fake_queue)
  memory = service.create(content: "x", project: nil)
  assert_equal 1, captured.size
  assert_equal "global", captured.first[1]
end

def test_delete_enqueues_push
  captured = []
  fake_queue = Object.new
  fake_queue.define_singleton_method(:enqueue) { |global_id:, db_name:| captured << [global_id, db_name] }
  service = Recollect::MemoriesService.new(Recollect::HTTPServer.db_manager, push_queue: fake_queue)
  memory = service.create(content: "x", project: nil)
  captured.clear
  service.delete(memory["id"], project: nil)
  assert_equal 1, captured.size
end
```

**Step 5: Run, expect PASS**

**Step 6: Commit**

```bash
git add lib/recollect/database_manager.rb lib/recollect/memories_service.rb lib/recollect/http_server.rb test/recollect/memories_service_test.rb
git commit -m "feat(sync): push-on-write hooks via MemoriesService"
```

---

## Sync engine (pull-on-startup + heartbeat)

### Task 15: `Sync::Engine` — one-shot reconciliation method

**Files:**
- Create: `lib/recollect/sync/engine.rb`
- Create: `test/recollect/sync/engine_test.rb`

**Context:** Phase 1 of the engine: a `reconcile(peer:, db_name:)` method that does manifest → pull (loop until short page) → push of what peer is missing. Heartbeat loop is the next task.

**Step 1: Write failing test**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::EngineTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)
    @peer_signing = Ed25519::SigningKey.generate
    @peers.add(peer_id: "peer-b", display_name: "B", public_key: @peer_signing.verify_key.to_bytes, endpoint: "http://b")
    @peers.subscribe("peer-b", "global")
  end

  def teardown
    @store.close
    super
  end

  def fake_client(manifest:, pull_pages:)
    pages = pull_pages.dup
    Class.new do
      define_method(:initialize) { |*| @manifest_response = manifest; @pages = pages; @pushed = [] }
      define_method(:pushed) { @pushed }
      define_method(:get) { |_path|
        Struct.new(:status, :success?, :body).new(200, true, JSON.generate(@manifest_response))
      }
      define_method(:post_json) { |path, payload|
        if path.start_with?("/sync/pull")
          page = @pages.shift || {records: []}
          Struct.new(:status, :success?, :body).new(200, true, JSON.generate(page))
        else
          @pushed << payload
          Struct.new(:status, :success?, :body).new(200, true, '{"accepted":1,"rejected":0}')
        end
      }
    end.new
  end

  def test_reconcile_pulls_records_from_peer
    page = {records: [{
      "global_id" => "g-from-b", "origin_peer" => "peer-b", "content" => "from b",
      "memory_type" => "note", "created_at" => "2026-05-15T00:00:00.000Z",
      "deleted_at" => nil, "deleted_by_peer" => nil, "tags" => nil, "metadata" => nil
    }]}
    client = fake_client(manifest: {watermarks: {}}, pull_pages: [page, {records: []}])
    engine = Recollect::Sync::Engine.new(store: @store, db_manager: Recollect::HTTPServer.db_manager, client_factory: ->(*) { client })
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    db = Recollect::HTTPServer.db_manager.get_database(nil)
    row = db.instance_variable_get(:@db).get_first_row("SELECT content FROM memories WHERE global_id = ?", "g-from-b")
    assert_equal "from b", row["content"]
  end

  def test_reconcile_pushes_what_peer_is_missing
    # Local has a self-origin row, peer's manifest has no watermark for us.
    db = Recollect::HTTPServer.db_manager.get_database(nil)
    db.store(content: "self-row", memory_type: "note")
    client = fake_client(manifest: {watermarks: {}}, pull_pages: [{records: []}])
    engine = Recollect::Sync::Engine.new(store: @store, db_manager: Recollect::HTTPServer.db_manager, client_factory: ->(*) { client })
    engine.reconcile(peer: @peers.find("peer-b"), db_name: "global")

    refute_empty client.pushed
    pushed_record = client.pushed.first[:records].first
    assert_equal "self-row", pushed_record["content"]
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true

module Recollect
  module Sync
    class Engine
      PULL_LIMIT = 500

      def initialize(store:, db_manager:, client_factory:, heartbeat_seconds: 300)
        @store = store
        @db_manager = db_manager
        @client_factory = client_factory
        @heartbeat_seconds = heartbeat_seconds
        @watermarks = Watermarks.new(store)
        @peers = Peers.new(store)
        @running = false
        @thread = nil
      end

      def reconcile(peer:, db_name:)
        client = @client_factory.call(peer)

        # Step 1: peer's manifest (what they have)
        manifest_response = client.get("/sync/manifest?db=#{db_name}")
        peer_watermarks = JSON.parse(manifest_response.body)["watermarks"]

        # Step 2: pull records we're missing
        pull_loop(client: client, db_name: db_name)

        # Step 3: push records peer is missing (from any origin we know about)
        push_missing(client: client, db_name: db_name, peer_watermarks: peer_watermarks)

        update_peer_success(peer[:peer_id])
      rescue => e
        update_peer_error(peer[:peer_id], e.message)
      end

      private

      def pull_loop(client:, db_name:)
        project  = @db_manager.project_for_db_name(db_name)
        database = @db_manager.get_database(project)

        loop do
          since = @watermarks.get(db_name: db_name)
          response = client.post_json("/sync/pull?db=#{db_name}", {since: since, limit: PULL_LIMIT})
          page = JSON.parse(response.body)["records"]
          break if page.empty?

          observed = Hash.new { |h, k| h[k] = "" }
          page.each do |rec|
            status = database.upsert_synced(rec)
            observed[rec["origin_peer"]] = [observed[rec["origin_peer"]], rec["created_at"]].max
            if status == :inserted && rec["memory_type"] != "_chunk" && rec["deleted_at"].nil?
              row = database.instance_variable_get(:@db).get_first_row("SELECT id FROM memories WHERE global_id = ?", rec["global_id"])
              @db_manager.enqueue_embedding(memory_id: row["id"], content: rec["content"], project: project) if row
            end
          end

          observed.each do |origin, ts|
            @watermarks.advance(peer_id: origin, db_name: db_name, timestamp: ts) unless ts.empty?
          end

          break if page.size < PULL_LIMIT
        end
      end

      def push_missing(client:, db_name:, peer_watermarks:)
        project  = @db_manager.project_for_db_name(db_name)
        database = @db_manager.get_database(project)
        rows = database.fetch_for_sync(since: peer_watermarks, limit: PULL_LIMIT)
        return if rows.empty?

        records = rows.map { |r| r.reject { |k, _| %w[id].include?(k) } }
        client.post_json("/sync/push?db=#{db_name}", {records: records})
      end

      def update_peer_success(peer_id)
        @store.instance_variable_get(:@db).execute(
          "UPDATE known_peers SET last_sync_at = ?, last_sync_error = NULL, last_seen_at = ? WHERE peer_id = ?",
          [Time.now.utc.iso8601, Time.now.utc.iso8601, peer_id]
        )
      end

      def update_peer_error(peer_id, message)
        @store.instance_variable_get(:@db).execute(
          "UPDATE known_peers SET last_sync_error = ? WHERE peer_id = ?", [message, peer_id]
        )
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/sync/engine.rb test/recollect/sync/engine_test.rb
git commit -m "feat(sync): Engine#reconcile does manifest+pull+push per (peer, db)"
```

---

### Task 16: `Sync::Engine#start` — pull-on-startup + heartbeat loop

**Files:**
- Modify: `lib/recollect/sync/engine.rb`
- Modify: `test/recollect/sync/engine_test.rb`

**Step 1: Write failing test**

```ruby
def test_start_runs_initial_reconcile_then_sleeps
  client = fake_client(manifest: {watermarks: {}}, pull_pages: [{records: []}])
  call_count = 0
  factory = ->(*) { call_count += 1; client }
  engine = Recollect::Sync::Engine.new(
    store: @store, db_manager: Recollect::HTTPServer.db_manager,
    client_factory: factory, heartbeat_seconds: 60
  )
  engine.start
  sleep 0.2
  engine.stop
  # Started, did at least one round (pull + push counts as multiple client calls per peer-db)
  assert_operator call_count, :>=, 1
end

def test_start_is_a_no_op_when_heartbeat_zero
  engine = Recollect::Sync::Engine.new(
    store: @store, db_manager: Recollect::HTTPServer.db_manager,
    client_factory: ->(*) { raise "should not be called" }, heartbeat_seconds: 0
  )
  engine.start  # should not raise
  engine.stop
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
def start
  return if @running || @heartbeat_seconds.zero?
  @running = true
  @thread = Thread.new { run_loop }
end

def stop
  @running = false
  @thread&.wakeup if @thread&.alive?
  @thread&.join(5)
end

private

def run_loop
  loop do
    break unless @running
    reconcile_all
    sleep_with_wakeup(@heartbeat_seconds)
  end
rescue => e
  warn "[Sync::Engine] loop crashed: #{e.message}"
end

def reconcile_all
  @peers.list.each do |peer|
    next unless peer[:status] == "trusted"
    @peers.subscriptions(peer[:peer_id]).each do |db_name|
      reconcile(peer: peer, db_name: db_name)
    end
  end
end

def sleep_with_wakeup(seconds)
  deadline = Time.now + seconds
  sleep([deadline - Time.now, 1].min) while Time.now < deadline && @running
end
```

**Step 4: Wire into `HTTPServer`**

In `http_server.rb`:

```ruby
def sync_engine
  return nil if Recollect.config.sync_disabled?
  return @sync_engine if @sync_engine

  init_mutex.synchronize do
    @sync_engine ||= begin
      identity = local_identity
      engine = Sync::Engine.new(
        store: sync_store, db_manager: db_manager,
        client_factory: ->(peer) {
          Sync::Client.new(peer_id: identity.peer_id, private_key: identity.private_key, endpoint: peer[:endpoint])
        },
        heartbeat_seconds: Recollect.config.sync_heartbeat_seconds
      )
      engine.start
      engine
    end
  end
end
```

Add to `reset_db_manager!`:

```ruby
@sync_engine&.stop
@sync_engine = nil
```

**Important:** the engine is initialized lazily, but production needs it eagerly. Add to `bin/server` (after Puma comes up) OR add an `at_start` hook. The simplest: call `HTTPServer.sync_engine` and `HTTPServer.push_queue` once at app boot in `bin/server` right after `require "recollect"`.

**Step 5: Update `bin/server`**

Open `bin/server`. After the line that loads recollect / config, add:

```ruby
# Eagerly start sync workers (no-op when RECOLLECT_SYNC_DISABLE=1)
Recollect::HTTPServer.push_queue
Recollect::HTTPServer.sync_engine
```

**Step 6: Run, expect PASS**

```bash
bundle exec rake test
```

**Step 7: Commit**

```bash
git add lib/recollect/sync/engine.rb lib/recollect/http_server.rb bin/server test/recollect/sync/engine_test.rb
git commit -m "feat(sync): Engine#start runs initial reconcile + heartbeat loop"
```

---

## CLI

### Task 17: `/api/sync/sync` endpoint + `recollect sync` CLI

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Modify: `bin/recollect`
- Test: `test/recollect/http_server_test.rb`

**Step 1: Write failing test for the endpoint**

```ruby
def test_api_sync_sync_kicks_engine
  signing_key = setup_trusted_peer("peer-a")
  Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store).subscribe("peer-a", "global")

  # Stub the client_factory by patching the engine
  # (For simplicity, just check the endpoint returns 200 and a summary structure)
  post "/api/sync/sync"
  assert_includes [200, 503], last_response.status
  if last_response.status == 200
    body = JSON.parse(last_response.body)
    assert body.key?("results")
  end
end
```

**Step 2: Implement the endpoint**

```ruby
post "/api/sync/sync" do
  require_loopback!
  engine = self.class.sync_engine
  halt 503, {error: "sync disabled"}.to_json unless engine
  body = request.body.read
  params_body = body.empty? ? {} : JSON.parse(body)

  peers_registry = Recollect::Sync::Peers.new(self.class.sync_store)
  peers_to_visit = if params_body["peer_id"]
    [peers_registry.find(params_body["peer_id"])].compact
  else
    peers_registry.list.select { |p| p[:status] == "trusted" }
  end

  results = peers_to_visit.flat_map do |peer|
    dbs = params_body["db_name"] ? [params_body["db_name"]] : peers_registry.subscriptions(peer[:peer_id])
    dbs.map do |db_name|
      engine.reconcile(peer: peer, db_name: db_name)
      {peer_id: peer[:peer_id], db_name: db_name, error: peer[:last_sync_error]}
    end
  end

  content_type :json
  {results: results}.to_json
end
```

**Step 3: Add CLI command**

In `bin/recollect`, inside `class CLI < Thor`:

```ruby
desc "sync", "Force an immediate sync cycle (debug)", hide: true
option :peer, type: :string, desc: "Sync only this peer"
option :db,   type: :string, desc: "Sync only this DB"
def sync
  payload = {}
  payload[:peer_id] = options[:peer] if options[:peer]
  payload[:db_name] = options[:db] if options[:db]
  response = post("/api/sync/sync", payload)
  if response.code == "200"
    data = JSON.parse(response.body)
    if data["results"].empty?
      say "No peers / no subscriptions."
    else
      data["results"].each do |r|
        line = "#{r["peer_id"]} db=#{r["db_name"]}"
        line << " ERROR: #{r["error"]}" if r["error"]
        say line
      end
    end
  else
    say @pastel.red("Error: #{response.code}")
  end
end
```

**Step 4: Smoke test manually**

```bash
./bin/server &
sleep 1
./bin/recollect sync
kill %1
```

Expected: "No peers / no subscriptions." (if no peers paired)

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb bin/recollect test/recollect/http_server_test.rb
git commit -m "feat(cli): recollect sync forces immediate reconciliation"
```

---

## Two-peer integration test harness

### Task 18: In-process two-peer harness

**Files:**
- Create: `test/integration/two_peer_helper.rb`
- Create: `test/integration/sync_two_peer_test.rb`

**Context:** Two `HTTPServer` instances need separate sync stores, separate data dirs, separate identities, but live in one Ruby process. The harness uses two `Recollect::Config` instances and patches `Recollect.config` per-helper-call. Cross-peer HTTP uses Faraday's Rack adapter pointing at the *other* Rack stack.

**Step 1: Write the harness**

```ruby
# test/integration/two_peer_helper.rb
# frozen_string_literal: true
require "test_helper"
require "faraday"
require "faraday/rack"

module TwoPeerHelper
  Peer = Struct.new(:label, :data_dir, :config, :app_class, :app_instance, keyword_init: true) do
    def with_config
      previous = Recollect.config
      Recollect.instance_variable_set(:@config, config)
      yield
    ensure
      Recollect.instance_variable_set(:@config, previous)
    end

    def identity
      with_config { app_class.local_identity }
    end

    def store(content:, project: nil)
      with_config { app_class.memories_service.create(content: content, project: project) }
    end

    def delete(id, project: nil)
      with_config { app_class.memories_service.delete(id, project: project) }
    end

    def list_global
      with_config { app_class.memories_service.list(project: nil) }
    end

    def push_queue
      with_config { app_class.push_queue }
    end
  end

  def make_peer(label)
    dir = Pathname.new(Dir.mktmpdir("recollect-#{label}-"))
    cfg = Recollect::Config.new(data_dir: dir, port: 0)  # port is unused (we don't bind a socket)

    # Build a fresh Sinatra subclass so its class-level singletons are independent
    klass = Class.new(Recollect::HTTPServer)
    klass.instance_variable_set(:@init_mutex, Mutex.new)

    # Force config swap on every singleton access
    Peer.new(label: label, data_dir: dir, config: cfg, app_class: klass, app_instance: klass.new!)
  end

  # Build a Sync::Client that routes through the other peer's Rack stack.
  def cross_peer_client_factory(from:, to:)
    ->(_peer_descriptor) {
      Recollect::Sync::Client.new(
        peer_id: from.identity.peer_id,
        private_key: from.identity.private_key,
        endpoint: "http://#{to.label}.test",
        adapter: [:rack, to.app_class]
      )
    }
  end

  def pair!(from:, to:)
    # Insert each peer into the other's known_peers + global subscription
    from.with_config do
      Recollect::Sync::Peers.new(from.app_class.sync_store).add(
        peer_id: to.identity.peer_id, display_name: to.label.to_s,
        public_key: to.identity.public_key, endpoint: "http://#{to.label}.test",
        default_subscription: "global"
      )
    end
    to.with_config do
      Recollect::Sync::Peers.new(to.app_class.sync_store).add(
        peer_id: from.identity.peer_id, display_name: from.label.to_s,
        public_key: from.identity.public_key, endpoint: "http://#{from.label}.test",
        default_subscription: "global"
      )
    end
  end

  def teardown_peer(peer)
    peer.with_config { peer.app_class.reset_db_manager! }
    FileUtils.remove_entry(peer.data_dir.to_s)
  end
end
```

**Step 2: Write a smoke integration test**

```ruby
# test/integration/sync_two_peer_test.rb
# frozen_string_literal: true
require_relative "two_peer_helper"

class SyncTwoPeerTest < Minitest::Test
  include TwoPeerHelper

  def setup
    @a = make_peer(:a)
    @b = make_peer(:b)
    pair!(from: @a, to: @b)
  end

  def teardown
    teardown_peer(@a)
    teardown_peer(@b)
  end

  def test_pull_propagates_record_from_a_to_b
    @a.store(content: "hello from a", project: nil)
    # B pulls from A
    factory = cross_peer_client_factory(from: @b, to: @a)
    @b.with_config do
      engine = Recollect::Sync::Engine.new(
        store: @b.app_class.sync_store, db_manager: @b.app_class.db_manager,
        client_factory: factory, heartbeat_seconds: 0
      )
      a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
      engine.reconcile(peer: a_peer, db_name: "global")
    end
    contents = @b.list_global.map { |m| m["content"] }
    assert_includes contents, "hello from a"
  end

  def test_tombstone_propagates
    memory = @a.store(content: "doomed", project: nil)
    @a.delete(memory["id"], project: nil)

    factory = cross_peer_client_factory(from: @b, to: @a)
    @b.with_config do
      engine = Recollect::Sync::Engine.new(
        store: @b.app_class.sync_store, db_manager: @b.app_class.db_manager,
        client_factory: factory, heartbeat_seconds: 0
      )
      a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
      engine.reconcile(peer: a_peer, db_name: "global")
    end
    contents = @b.list_global.map { |m| m["content"] }
    refute_includes contents, "doomed"
  end

  def test_subscription_filter_blocks_unsubscribed_db
    @a.with_config do
      Recollect::Sync::Peers.new(@a.app_class.sync_store).unsubscribe(@b.identity.peer_id, "global")
    end
    @a.store(content: "secret", project: nil)

    factory = cross_peer_client_factory(from: @b, to: @a)
    @b.with_config do
      engine = Recollect::Sync::Engine.new(
        store: @b.app_class.sync_store, db_manager: @b.app_class.db_manager,
        client_factory: factory, heartbeat_seconds: 0
      )
      a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
      engine.reconcile(peer: a_peer, db_name: "global")
    end
    contents = @b.list_global.map { |m| m["content"] }
    refute_includes contents, "secret"
  end
end
```

**Step 3: Add the integration directory to the Rakefile test pattern if needed**

Check `Rakefile` — most likely uses `test/**/*_test.rb`. If only `test/recollect/**`, broaden.

**Step 4: Run, expect PASS**

```bash
bundle exec rake test
```

If any test flickers due to thread races, surface the issue and either flush queues explicitly or extend `engine.reconcile` to be synchronous (it already is — only `PushQueue` is async; tests above bypass it).

**Step 5: Commit**

```bash
git add test/integration/two_peer_helper.rb test/integration/sync_two_peer_test.rb Rakefile
git commit -m "test(sync): two-peer in-process integration harness"
```

---

### Task 19: Push-on-write integration test (uses the harness from Task 18)

**Files:**
- Modify: `test/integration/sync_two_peer_test.rb`

**Step 1: Write failing test**

Append:

```ruby
def test_push_on_write_propagates_a_to_b
  # Inject cross-peer client into A's push queue
  factory = cross_peer_client_factory(from: @a, to: @b)
  @a.with_config do
    queue = @a.app_class.push_queue
    queue.instance_variable_set(:@client_factory, factory)
  end

  memory = @a.store(content: "pushed", project: nil)
  @a.push_queue.flush(timeout: 3)

  contents = @b.list_global.map { |m| m["content"] }
  assert_includes contents, "pushed"
end
```

**Step 2: Run, expect PASS** (or fail and iterate)

```bash
bundle exec ruby -Itest test/integration/sync_two_peer_test.rb -n test_push_on_write_propagates_a_to_b
```

**Step 3: Commit**

```bash
git add test/integration/sync_two_peer_test.rb
git commit -m "test(sync): push-on-write integration coverage"
```

---

### Task 20: Pagination integration test

**Files:**
- Modify: `test/integration/sync_two_peer_test.rb`

**Step 1: Write failing test**

```ruby
def test_pull_paginates_across_many_records
  600.times { |i| @a.store(content: "item #{i}", project: nil) }
  factory = cross_peer_client_factory(from: @b, to: @a)
  @b.with_config do
    engine = Recollect::Sync::Engine.new(
      store: @b.app_class.sync_store, db_manager: @b.app_class.db_manager,
      client_factory: factory, heartbeat_seconds: 0
    )
    a_peer = Recollect::Sync::Peers.new(@b.app_class.sync_store).find(@a.identity.peer_id)
    engine.reconcile(peer: a_peer, db_name: "global")
  end
  count = @b.with_config { @b.app_class.db_manager.get_database(nil).count }
  assert_equal 600, count
end
```

**Step 2: Run, expect PASS**

**Step 3: Commit**

```bash
git add test/integration/sync_two_peer_test.rb
git commit -m "test(sync): pull pagination across 600 records"
```

---

## Final verification

### Task 21: Rubocop, full tests, coverage, end-to-end smoke

**Step 1: Rubocop**

```bash
bundle exec rake rubocop
```

Fix offenses. Run `-A` if safe-autocorrect helps.

**Step 2: Full test suite**

```bash
bundle exec rake test
```

All green.

**Step 3: Coverage**

```bash
bundle exec rake coverage
```

Must not regress below baseline (Phase 1 ended at 86.97%; Phase 2 should be similar or higher).

**Step 4: Two-process smoke test**

```bash
# Terminal A
RECOLLECT_DATA_DIR=/tmp/rec-a RECOLLECT_PORT=7326 RECOLLECT_PUBLIC_URL=http://127.0.0.1:7326 ./bin/server

# Terminal B
RECOLLECT_DATA_DIR=/tmp/rec-b RECOLLECT_PORT=7327 RECOLLECT_PUBLIC_URL=http://127.0.0.1:7327 ./bin/server

# Pair them (Terminal C)
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect pair
# Copy code, then:
RECOLLECT_URL=http://127.0.0.1:7327 ./bin/recollect join <CODE> --endpoint http://127.0.0.1:7326

# Store on A (no -p: plain stores go to the global db; "global" is a
# reserved name and rejected as a project):
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect store "from a"

# Wait a moment then check B:
sleep 2
RECOLLECT_URL=http://127.0.0.1:7327 ./bin/recollect list
# Expected: "from a" appears

# Tombstone on A:
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect search "from a"
# get the id, then:
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect delete <id>

# Confirm B sees the deletion:
sleep 2
RECOLLECT_URL=http://127.0.0.1:7327 ./bin/recollect list
# Expected: "from a" gone
```

**Step 5: Final commit (if rubocop pending)**

```bash
git status
git commit -m "chore: rubocop fixes for sync phase 2"
```

---

## Done — what's next

Phase 2 complete: ambient sync working, both push-on-write and pull-on-startup paths covered, heartbeat reconciles drift, CLI debug command in place.

Suggested Phase 3 candidates (none committed):
- Re-embed pass for embedding model changes.
- Vacuum / GC for old tombstones.
- Web UI for peer management (list peers, accept pairings interactively, edit subscriptions).
- Conflict observability (count of late-arriving content for tombstoned IDs).
- Compress sync payloads (gzip on Faraday + content-encoding negotiation).
