# Recollect Sync — Phase 1 (Foundation) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land all schema, identity, crypto, pairing, and CLI scaffolding for the Recollect sync feature, with no network sync behavior yet. After Phase 1, two instances can pair via CLI and see each other in `recollect peers`, but no records flow.

**Architecture:** Add three columns (`global_id`, `origin_peer`, `deleted_at`, `deleted_by_peer`) to memory tables. Introduce a new `~/.recollect/sync.db` (chmod 0600) holding peer identity, trust state, watermarks, pairing codes, and outbound sharing policy. Generate an Ed25519 keypair on first run. Add `/pairing/create` (local-only) and `/pairing/join` HTTP endpoints. Add CLI commands: `identity`, `pair`, `join`, `peers (list/remove/add-db/remove-db)`. Switch existing `delete_memory` from hard-delete to tombstone (`deleted_at`).

**Tech Stack:** Ruby 3.4+ (UUIDv7 via `SecureRandom.uuid_v7`), SQLite3 (sqlite3 gem), Sinatra/Puma (HTTP), Thor (CLI), `ed25519` gem (signatures), `base58` for peer_id encoding. Tests with Minitest + Rack::Test.

**Reference:** [Sync Design](2026-05-02-sync-design.md). The design doc is the source of truth for the *what*; this plan is the *how*.

**Branch:** `sync/phase-1` off `master`.

**Out of scope (this phase):** all `/sync/*` endpoints, push-on-write, pull-on-startup, heartbeat, embedding sync, `recollect sync` debug command. These are Phase 2.

**Open questions deferred to Phase 2 design:**
- How chunks (`memory_type='_chunk'`) are handled when records sync. `parent_id` in chunk metadata is a local integer id and isn't portable. Three options: (a) don't sync chunks, receiver re-chunks; (b) sync chunks with `parent_id` ↔ `parent_global_id` translation at the boundary; (c) refactor chunks to use `parent_global_id` natively. Phase 1 schema is unaffected — chunks get `global_id` like everything else.

---

## Setup

### Task 0: Branch and dependencies

**Files:**
- Modify: `Gemfile`

**Step 1: Create branch**

```bash
git checkout -b sync/phase-1
```

**Step 2: Add ed25519 and base58 gems**

In `Gemfile`, add to the `# Utilities` section:

```ruby
gem "ed25519", "~> 1.3"
gem "base58", "~> 0.2"
```

**Step 3: Install**

```bash
bundle install
```

**Step 4: Verify Ruby has UUIDv7**

```bash
bundle exec ruby -rsecurerandom -e 'puts SecureRandom.uuid_v7'
```

Expected: a 36-char UUID printed (e.g., `01918a9b-...`).

**Step 5: Commit**

```bash
git add Gemfile Gemfile.lock
git commit -m "chore: add ed25519 and base58 gems for sync"
```

---

## Schema and Tombstones (no identity yet — global_id and origin_peer backfill comes later)

### Task 1: Add sync columns to `memories` table (nullable, no backfill)

**Files:**
- Modify: `lib/recollect/database.rb` (the `SCHEMA` constant and `create_schema` method)
- Test: `test/recollect/database_test.rb`

**Context:** The schema needs four new nullable columns. Existing rows will be backfilled in a later task once we have a local `peer_id`. We use SQLite `ALTER TABLE` to keep the migration idempotent for existing DBs.

**Step 1: Write failing test**

Append to `test/recollect/database_test.rb`:

```ruby
def test_memories_table_has_sync_columns
  cols = @db.instance_variable_get(:@db).execute("PRAGMA table_info(memories)")
  names = cols.map { |c| c["name"] }
  assert_includes names, "global_id"
  assert_includes names, "origin_peer"
  assert_includes names, "deleted_at"
  assert_includes names, "deleted_by_peer"
end

def test_global_id_is_unique_when_present
  @db.instance_variable_get(:@db).execute(
    "INSERT INTO memories (content, memory_type, global_id) VALUES (?, ?, ?)",
    ["a", "note", "uuid-x"]
  )
  assert_raises(SQLite3::ConstraintException) do
    @db.instance_variable_get(:@db).execute(
      "INSERT INTO memories (content, memory_type, global_id) VALUES (?, ?, ?)",
      ["b", "note", "uuid-x"]
    )
  end
end
```

**Step 2: Run test, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /sync_columns|global_id_is_unique/
```

Expected: failures on both — columns missing.

**Step 3: Implement migration**

In `lib/recollect/database.rb`, after `create_schema` is called in `initialize`, add a `migrate_sync_columns` step. Implement as:

```ruby
def migrate_sync_columns
  cols = @db.execute("PRAGMA table_info(memories)").map { |c| c["name"] }
  unless cols.include?("global_id")
    @db.execute("ALTER TABLE memories ADD COLUMN global_id TEXT")
  end
  unless cols.include?("origin_peer")
    @db.execute("ALTER TABLE memories ADD COLUMN origin_peer TEXT")
  end
  unless cols.include?("deleted_at")
    @db.execute("ALTER TABLE memories ADD COLUMN deleted_at TEXT")
  end
  unless cols.include?("deleted_by_peer")
    @db.execute("ALTER TABLE memories ADD COLUMN deleted_by_peer TEXT")
  end

  @db.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_global_id ON memories(global_id) WHERE global_id IS NOT NULL")
  @db.execute("CREATE INDEX IF NOT EXISTS idx_memories_origin_created ON memories(origin_peer, created_at)")
  @db.execute("CREATE INDEX IF NOT EXISTS idx_memories_deleted ON memories(deleted_at) WHERE deleted_at IS NOT NULL")
end
```

Call `migrate_sync_columns` from `initialize`, after `create_schema`.

**Step 4: Run test, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /sync_columns|global_id_is_unique/
```

Expected: PASS.

**Step 5: Run full test suite — nothing else should break**

```bash
bundle exec rake test
```

Expected: all green.

**Step 6: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): add nullable sync columns and indexes to memories"
```

---

### Task 2: Switch `Database#store` to populate `global_id` and `origin_peer`

**Files:**
- Modify: `lib/recollect/database.rb`
- Modify: `lib/recollect/database_manager.rb` (so it knows the local peer_id; for now pass through)
- Test: `test/recollect/database_test.rb`

**Context:** Every newly inserted row gets a fresh UUIDv7 and the local peer_id as `origin_peer`. Until identity exists, accept a peer_id arg with a fallback to a static placeholder `"local"` so tests pass independently. Real identity wired in Task 11.

**Step 1: Write failing test**

```ruby
def test_store_assigns_global_id_and_origin_peer
  id = @db.store(content: "hello", memory_type: "note", origin_peer: "test-peer")
  row = @db.instance_variable_get(:@db).get_first_row("SELECT global_id, origin_peer FROM memories WHERE id = ?", id)
  assert_match(/\A[0-9a-f-]{36}\z/, row["global_id"])
  assert_equal "test-peer", row["origin_peer"]
end

def test_store_default_origin_peer_is_local
  id = @db.store(content: "hello", memory_type: "note")
  row = @db.instance_variable_get(:@db).get_first_row("SELECT origin_peer FROM memories WHERE id = ?", id)
  assert_equal "local", row["origin_peer"]
end
```

**Step 2: Run test, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /assigns_global_id|default_origin_peer/
```

**Step 3: Implement**

Update `Database#store` signature and body:

```ruby
def store(content:, memory_type: "note", tags: nil, metadata: nil, origin_peer: "local")
  raise ArgumentError, "content cannot be empty" if content.nil? || content.to_s.strip.empty?
  normalized_tags = tags&.map(&:downcase)
  global_id = SecureRandom.uuid_v7
  @db.execute(
    "INSERT INTO memories (content, memory_type, tags, metadata, global_id, origin_peer) VALUES (?, ?, ?, ?, ?, ?)",
    [content, memory_type, json_encode(normalized_tags), json_encode(metadata), global_id, origin_peer]
  )
  @db.last_insert_row_id
end
```

Add `require "securerandom"` at the top of `database.rb` if not already present.

**Step 4: Run targeted tests, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /assigns_global_id|default_origin_peer/
```

**Step 5: Run full suite**

```bash
bundle exec rake test
```

Expected: all green. (Existing tests don't pass `origin_peer`; they get `"local"` by default.)

**Step 6: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): populate global_id and origin_peer on insert"
```

---

### Task 3: Tombstone trigger for FTS (always-on)

**Files:**
- Modify: `lib/recollect/database.rb` (extend `migrate_sync_columns` to install the trigger)
- Test: `test/recollect/database_test.rb`

**Step 1: Write failing test**

```ruby
def test_tombstone_removes_row_from_fts
  id = @db.store(content: "haystack needle marker", memory_type: "note")
  # Confirm searchable
  refute_empty @db.search("marker")
  # Tombstone
  @db.instance_variable_get(:@db).execute(
    "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
    [Time.now.utc.iso8601, "local", id]
  )
  # FTS row gone
  fts_count = @db.instance_variable_get(:@db).get_first_value(
    "SELECT COUNT(*) FROM memories_fts WHERE rowid = ?", id
  )
  assert_equal 0, fts_count
end
```

**Step 2: Run test, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n test_tombstone_removes_row_from_fts
```

**Step 3: Implement trigger**

Extend `migrate_sync_columns` with:

```ruby
@db.execute(<<~SQL)
  CREATE TRIGGER IF NOT EXISTS memories_tombstone_fts
  AFTER UPDATE ON memories
  WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL
  BEGIN
    DELETE FROM memories_fts WHERE rowid = NEW.id;
  END;
SQL
```

**Step 4: Run test, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n test_tombstone_removes_row_from_fts
```

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): FTS tombstone trigger removes deleted rows from index"
```

---

### Task 4: Tombstone trigger for vector index (installed with extension)

**Files:**
- Modify: `lib/recollect/database.rb` (`load_vector_extension` method)
- Test: `test/recollect/database_test.rb` (skip when vectors not available)

**Step 1: Write failing test**

```ruby
def test_tombstone_removes_row_from_vec_when_vectors_loaded
  skip "vectors not available" unless @db.vectors_enabled?
  id = @db.store(content: "x", memory_type: "note")
  raw = @db.instance_variable_get(:@db)
  # Insert a fake embedding row
  raw.execute("INSERT INTO vec_memories(rowid, embedding) VALUES (?, ?)", [id, "\0" * (4 * 384)])
  assert_equal 1, raw.get_first_value("SELECT COUNT(*) FROM vec_memories WHERE rowid = ?", id)
  # Tombstone
  raw.execute("UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
              [Time.now.utc.iso8601, "local", id])
  assert_equal 0, raw.get_first_value("SELECT COUNT(*) FROM vec_memories WHERE rowid = ?", id)
end
```

(If your test setup doesn't load vectors by default, this test will skip — that's fine; the trigger still gets exercised in integration.)

**Step 2: Run test**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n test_tombstone_removes_row_from_vec_when_vectors_loaded
```

If your test env has vectors: expect FAIL. If not: expect SKIP.

**Step 3: Implement trigger inside `load_vector_extension`**

In `Database#load_vector_extension`, after `@db.execute_batch(VECTOR_SCHEMA)`:

```ruby
@db.execute(<<~SQL)
  CREATE TRIGGER IF NOT EXISTS memories_tombstone_vec
  AFTER UPDATE ON memories
  WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL
  BEGIN
    DELETE FROM vec_memories WHERE rowid = NEW.id;
  END;
SQL
```

**Step 4: Run test, expect PASS (or SKIP)**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n test_tombstone_removes_row_from_vec_when_vectors_loaded
```

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): vec tombstone trigger installed with sqlite-vec extension"
```

---

### Task 5: Filter tombstoned rows from `get`, `list`, `search`, `search_by_tags`, `vector_search`

**Files:**
- Modify: `lib/recollect/database.rb`
- Test: `test/recollect/database_test.rb`

**Context:** Add `AND deleted_at IS NULL` (or `WHERE deleted_at IS NULL` where appropriate) to every read path on the `memories` table.

**Step 1: Write failing tests**

```ruby
def test_get_returns_nil_for_tombstoned
  id = @db.store(content: "x", memory_type: "note")
  @db.instance_variable_get(:@db).execute(
    "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
    [Time.now.utc.iso8601, "local", id]
  )
  assert_nil @db.get(id)
end

def test_list_excludes_tombstoned
  alive = @db.store(content: "alive", memory_type: "note")
  dead  = @db.store(content: "dead",  memory_type: "note")
  @db.instance_variable_get(:@db).execute(
    "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
    [Time.now.utc.iso8601, "local", dead]
  )
  ids = @db.list.map { |r| r["id"] }
  assert_includes ids, alive
  refute_includes ids, dead
end

def test_search_excludes_tombstoned
  alive = @db.store(content: "needle alive", memory_type: "note")
  dead  = @db.store(content: "needle dead",  memory_type: "note")
  @db.instance_variable_get(:@db).execute(
    "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ?",
    [Time.now.utc.iso8601, "local", dead]
  )
  ids = @db.search("needle").map { |r| r["id"] }
  assert_includes ids, alive
  refute_includes ids, dead
end
```

(Note: `test_search_excludes_tombstoned` should also pass purely via the FTS trigger from Task 3 — but the explicit `WHERE deleted_at IS NULL` is belt-and-braces in case some path bypasses FTS.)

**Step 2: Run tests, expect at least `get` and `list` to FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /tombstoned|excludes_tombstoned/
```

**Step 3: Add the filter**

Audit every method in `Database` that reads `memories`:
- `get(id)`: add `AND m.deleted_at IS NULL` (vectors path) and `AND deleted_at IS NULL` (non-vectors path).
- `list`: add `AND m.deleted_at IS NULL` / `AND deleted_at IS NULL`.
- `search` (both wildcard and FTS branches): add `AND memories.deleted_at IS NULL`.
- `search_by_tags`: add `AND deleted_at IS NULL`.
- `vector_search`: add `AND m.deleted_at IS NULL`.
- `get_tag_stats`: add `AND deleted_at IS NULL`.

Read each method, add the filter once per method.

**Step 4: Run tests, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /tombstoned|excludes_tombstoned/
bundle exec rake test
```

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): exclude tombstoned rows from all read queries"
```

---

### Task 6: Switch `Database#delete` from hard-delete to tombstone

**Files:**
- Modify: `lib/recollect/database.rb` (`delete` method)
- Test: `test/recollect/database_test.rb`

**Context:** `delete` currently issues `DELETE FROM memories`. Change it to UPDATE setting `deleted_at = now`, `deleted_by_peer = origin_peer arg`. The triggers and read filters do the rest.

**Step 1: Write failing test**

```ruby
def test_delete_tombstones_row_instead_of_hard_delete
  id = @db.store(content: "doomed", memory_type: "note")
  @db.delete(id, deleted_by_peer: "local")
  raw = @db.instance_variable_get(:@db)
  row = raw.get_first_row("SELECT id, content, deleted_at, deleted_by_peer FROM memories WHERE id = ?", id)
  refute_nil row, "row should still exist physically (tombstone, not hard delete)"
  refute_nil row["deleted_at"], "deleted_at should be set"
  assert_equal "local", row["deleted_by_peer"]
end

def test_delete_returns_true_when_row_existed
  id = @db.store(content: "x", memory_type: "note")
  assert_equal true, @db.delete(id, deleted_by_peer: "local")
end

def test_delete_returns_false_when_row_missing
  assert_equal false, @db.delete(99_999, deleted_by_peer: "local")
end

def test_delete_idempotent_on_already_deleted
  id = @db.store(content: "x", memory_type: "note")
  @db.delete(id, deleted_by_peer: "local")
  # Second call should not raise; should be no-op (returns false because deleted_at is unchanged)
  assert_equal false, @db.delete(id, deleted_by_peer: "local")
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /test_delete_/
```

**Step 3: Implement**

Replace `Database#delete`:

```ruby
def delete(id, deleted_by_peer: "local")
  result = @db.execute(
    "UPDATE memories SET deleted_at = ?, deleted_by_peer = ? WHERE id = ? AND deleted_at IS NULL",
    [Time.now.utc.iso8601(3), deleted_by_peer, id]
  )
  @db.changes.positive?
end
```

(Returns `true` if a row was tombstoned, `false` otherwise — matches existing semantics for "did anything happen.")

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/database_test.rb -n /test_delete_/
bundle exec rake test
```

Existing callers (`MemoriesService#delete`, `delete_memory` tool) currently call `db.delete(id)` without kwargs — they'll get `"local"` as default, which is fine until identity is wired in.

**Step 5: Commit**

```bash
git add lib/recollect/database.rb test/recollect/database_test.rb
git commit -m "feat(db): delete now tombstones rather than hard-deletes"
```

---

## sync.db and Identity

### Task 7: Add `Recollect::Sync::Store` (sync.db schema and lifecycle)

**Files:**
- Create: `lib/recollect/sync/store.rb`
- Create: `test/recollect/sync/store_test.rb`
- Modify: `lib/recollect/config.rb` (add `sync_db_path`)

**Context:** `Sync::Store` owns `~/.recollect/sync.db`. Mirror the pattern of `Database` — wraps an SQLite3 connection, applies schema on init, exposes typed accessor methods. File mode 0600 set after creation.

**Step 1: Write failing test**

Create `test/recollect/sync/store_test.rb`:

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::StoreTest < Recollect::TestCase
  def setup
    super
    @path = Recollect.config.data_dir.join("sync.db")
    @store = Recollect::Sync::Store.new(@path)
  end

  def teardown
    @store.close
    super
  end

  def test_creates_all_tables
    raw = @store.instance_variable_get(:@db)
    tables = raw.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").map { |r| r["name"] }
    %w[local_identity known_peers peer_watermarks pairing_codes peer_db_subscriptions].each do |t|
      assert_includes tables, t
    end
  end

  def test_file_mode_is_0600
    File.chmod(0o644, @path) # set wrong mode first
    @store.close
    @store = Recollect::Sync::Store.new(@path)
    mode = File.stat(@path).mode & 0o777
    assert_equal 0o600, mode
  end

  def test_idempotent_open
    @store.close
    again = Recollect::Sync::Store.new(@path)
    refute_nil again.instance_variable_get(:@db)
    again.close
  end
end
```

**Step 2: Run, expect FAIL (no Store class)**

```bash
bundle exec ruby -Itest test/recollect/sync/store_test.rb
```

**Step 3: Add `sync_db_path` to Config**

In `lib/recollect/config.rb`, add a method:

```ruby
def sync_db_path
  data_dir.join("sync.db")
end
```

**Step 4: Implement Store**

Create `lib/recollect/sync/store.rb`:

```ruby
# frozen_string_literal: true
require "sqlite3"

module Recollect
  module Sync
    class Store
      SCHEMA = <<~SQL
        CREATE TABLE IF NOT EXISTS local_identity (
          peer_id      TEXT PRIMARY KEY,
          display_name TEXT,
          public_key   BLOB NOT NULL,
          private_key  BLOB NOT NULL,
          created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        );

        CREATE TABLE IF NOT EXISTS known_peers (
          peer_id         TEXT PRIMARY KEY,
          display_name    TEXT,
          public_key      BLOB NOT NULL,
          endpoint        TEXT NOT NULL,
          status          TEXT NOT NULL DEFAULT 'trusted',
          trusted_at      TEXT NOT NULL,
          last_seen_at    TEXT,
          last_sync_at    TEXT,
          last_sync_error TEXT
        );

        CREATE TABLE IF NOT EXISTS peer_watermarks (
          peer_id           TEXT NOT NULL,
          db_name           TEXT NOT NULL,
          latest_created_at TEXT NOT NULL,
          PRIMARY KEY (peer_id, db_name)
        );

        CREATE TABLE IF NOT EXISTS pairing_codes (
          code         TEXT PRIMARY KEY,
          created_at   TEXT NOT NULL,
          expires_at   TEXT NOT NULL,
          used_at      TEXT,
          used_by_peer TEXT
        );

        CREATE TABLE IF NOT EXISTS peer_db_subscriptions (
          peer_id TEXT NOT NULL,
          db_name TEXT NOT NULL,
          PRIMARY KEY (peer_id, db_name)
        );
      SQL

      def initialize(path)
        @path = path.to_s
        FileUtils.mkdir_p(File.dirname(@path))
        @db = SQLite3::Database.new(@path)
        @db.results_as_hash = true
        @db.execute("PRAGMA journal_mode = WAL")
        @db.execute("PRAGMA foreign_keys = ON")
        @db.execute_batch(SCHEMA)
        File.chmod(0o600, @path)
      end

      def close
        @db&.close
        @db = nil
      end
    end
  end
end
```

**Step 5: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/store_test.rb
```

**Step 6: Commit**

```bash
git add lib/recollect/sync/store.rb test/recollect/sync/store_test.rb lib/recollect/config.rb
git commit -m "feat(sync): add Sync::Store with sync.db schema and 0600 perms"
```

---

### Task 8: Identity generation (Ed25519 keypair, peer_id derivation)

**Files:**
- Create: `lib/recollect/sync/identity.rb`
- Create: `test/recollect/sync/identity_test.rb`
- Modify: `lib/recollect/sync/store.rb` (add `local_identity` accessors)

**Context:** On first call, generate Ed25519 keypair, derive `peer_id = base58(sha256(public_key)[0,16])`, store in `local_identity` row. On subsequent calls, read existing identity. Idempotent.

**Step 1: Write failing test**

Create `test/recollect/sync/identity_test.rb`:

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::IdentityTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
  end

  def teardown
    @store.close
    super
  end

  def test_first_call_generates_keypair
    identity = Recollect::Sync::Identity.ensure!(@store, display_name: "test-host")
    assert_match(/\A[1-9A-HJ-NP-Za-km-z]+\z/, identity.peer_id, "peer_id should be base58")
    refute_nil identity.public_key
    refute_nil identity.private_key
    assert_equal "test-host", identity.display_name
  end

  def test_second_call_returns_same_identity
    first  = Recollect::Sync::Identity.ensure!(@store, display_name: "test-host")
    second = Recollect::Sync::Identity.ensure!(@store, display_name: "ignored")
    assert_equal first.peer_id, second.peer_id
    assert_equal first.public_key, second.public_key
    assert_equal "test-host", second.display_name, "display_name from first call wins"
  end

  def test_peer_id_derivation_is_deterministic
    pub = "\x00" * 32
    expected = Recollect::Sync::Identity.derive_peer_id(pub)
    assert_equal expected, Recollect::Sync::Identity.derive_peer_id(pub)
  end

  def test_can_sign_and_verify_with_keypair
    identity = Recollect::Sync::Identity.ensure!(@store)
    signing_key  = Ed25519::SigningKey.new(identity.private_key)
    verify_key   = Ed25519::VerifyKey.new(identity.public_key)
    sig = signing_key.sign("hello")
    assert verify_key.verify(sig, "hello")
  end
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/sync/identity_test.rb
```

**Step 3: Implement Identity**

Create `lib/recollect/sync/identity.rb`:

```ruby
# frozen_string_literal: true
require "ed25519"
require "base58"
require "digest"
require "socket"

module Recollect
  module Sync
    class Identity
      attr_reader :peer_id, :display_name, :public_key, :private_key, :created_at

      def initialize(peer_id:, display_name:, public_key:, private_key:, created_at:)
        @peer_id      = peer_id
        @display_name = display_name
        @public_key   = public_key
        @private_key  = private_key
        @created_at   = created_at
      end

      def self.ensure!(store, display_name: nil)
        existing = load(store)
        return existing if existing

        signing_key = Ed25519::SigningKey.generate
        public_key  = signing_key.verify_key.to_bytes
        private_key = signing_key.to_bytes
        peer_id     = derive_peer_id(public_key)
        name        = display_name || Socket.gethostname

        store.instance_variable_get(:@db).execute(
          "INSERT INTO local_identity (peer_id, display_name, public_key, private_key) VALUES (?, ?, ?, ?)",
          [peer_id, name, SQLite3::Blob.new(public_key), SQLite3::Blob.new(private_key)]
        )
        load(store)
      end

      def self.load(store)
        row = store.instance_variable_get(:@db).get_first_row("SELECT * FROM local_identity LIMIT 1")
        return nil unless row
        new(
          peer_id:      row["peer_id"],
          display_name: row["display_name"],
          public_key:   row["public_key"],
          private_key:  row["private_key"],
          created_at:   row["created_at"]
        )
      end

      def self.derive_peer_id(public_key)
        Base58.binary_to_base58(Digest::SHA256.digest(public_key)[0, 16].b)
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/identity_test.rb
```

**Step 5: Commit**

```bash
git add lib/recollect/sync/identity.rb test/recollect/sync/identity_test.rb
git commit -m "feat(sync): generate and persist Ed25519 identity with base58 peer_id"
```

---

### Task 9: Wire Identity into HTTPServer singleton + backfill memory rows

**Files:**
- Modify: `lib/recollect/http_server.rb` (add `sync_store` and `local_identity` singletons)
- Modify: `lib/recollect/database.rb` (add a `backfill_origin_peer!` method)
- Modify: `lib/recollect/database_manager.rb` (run backfill on first access; pass real peer_id to `store`)
- Test: `test/recollect/http_server_test.rb` and `test/recollect/database_test.rb`

**Context:** Now that we can generate identity, every memory DB needs to know the local peer_id (for new inserts and for backfilling existing rows). The `DatabaseManager` is the natural place to hold the local peer_id and pass it to each `Database`.

**Step 1: Write failing test for HTTPServer singleton**

In `test/recollect/http_server_test.rb`:

```ruby
def test_sync_store_singleton_is_initialized
  assert_kind_of Recollect::Sync::Store, Recollect::HTTPServer.sync_store
end

def test_local_identity_is_available
  identity = Recollect::HTTPServer.local_identity
  assert_kind_of Recollect::Sync::Identity, identity
  refute_nil identity.peer_id
end
```

**Step 2: Write failing test for backfill**

In `test/recollect/database_test.rb`:

```ruby
def test_backfill_origin_peer_fills_null_rows
  raw = @db.instance_variable_get(:@db)
  # Insert a row that bypasses the normal store path (simulating legacy data)
  raw.execute("INSERT INTO memories (content, memory_type) VALUES ('legacy', 'note')")
  legacy_id = raw.last_insert_row_id

  @db.backfill_origin_peer!("peer-abc")
  row = raw.get_first_row("SELECT global_id, origin_peer FROM memories WHERE id = ?", legacy_id)
  refute_nil row["global_id"]
  assert_equal "peer-abc", row["origin_peer"]
end

def test_backfill_does_not_clobber_existing_origin_peer
  id = @db.store(content: "x", memory_type: "note", origin_peer: "other-peer")
  @db.backfill_origin_peer!("local-peer")
  row = @db.instance_variable_get(:@db).get_first_row("SELECT origin_peer FROM memories WHERE id = ?", id)
  assert_equal "other-peer", row["origin_peer"]
end
```

**Step 3: Run, expect FAIL on all four**

```bash
bundle exec rake test
```

**Step 4: Implement Database#backfill_origin_peer!**

```ruby
def backfill_origin_peer!(local_peer_id)
  rows = @db.execute("SELECT id FROM memories WHERE global_id IS NULL OR origin_peer IS NULL")
  rows.each do |row|
    @db.execute(
      "UPDATE memories SET global_id = COALESCE(global_id, ?), origin_peer = COALESCE(origin_peer, ?) WHERE id = ?",
      [SecureRandom.uuid_v7, local_peer_id, row["id"]]
    )
  end
end
```

**Step 5: Wire into DatabaseManager**

Add `local_peer_id` to `DatabaseManager#initialize` (kwarg, optional). When `get_database` creates a new `Database`, call `db.backfill_origin_peer!(local_peer_id)` if `local_peer_id` present.

`DatabaseManager#store_with_embedding` and any direct `db.store` calls should pass `origin_peer: @local_peer_id`. Update `store_with_embedding` signature:

```ruby
def store_with_embedding(project:, content:, memory_type:, tags:, metadata:)
  db = get_database(project)
  parent_id = db.store(content: content, memory_type: memory_type, tags: tags, metadata: metadata, origin_peer: @local_peer_id || "local")
  # ... rest unchanged, but chunks also pass origin_peer:
end
```

**Step 6: Wire into HTTPServer**

Add class-level singletons mirroring the existing `db_manager` pattern:

```ruby
class HTTPServer < Sinatra::Base
  class << self
    def sync_store
      @sync_store ||= Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    end

    def local_identity
      @local_identity ||= Recollect::Sync::Identity.ensure!(sync_store)
    end

    def db_manager
      @db_manager ||= Recollect::DatabaseManager.new(Recollect.config, local_peer_id: local_identity.peer_id)
    end

    def reset_db_manager!
      @db_manager&.close_all
      @db_manager = nil
      @sync_store&.close
      @sync_store = nil
      @local_identity = nil
    end
  end
end
```

(Update `DatabaseManager#initialize` to accept `local_peer_id:` kwarg. Default `nil`.)

**Step 7: Run all tests, expect PASS**

```bash
bundle exec rake test
```

**Step 8: Commit**

```bash
git add lib/recollect/http_server.rb lib/recollect/database.rb lib/recollect/database_manager.rb test/recollect/http_server_test.rb test/recollect/database_test.rb
git commit -m "feat(sync): wire Identity into HTTPServer + backfill memory rows on first open"
```

---

## Crypto and Endpoint Discovery

### Task 10: Request signing and verification helpers

**Files:**
- Create: `lib/recollect/sync/crypto.rb`
- Create: `test/recollect/sync/crypto_test.rb`

**Context:** Two pure-functional helpers. `Crypto.sign(private_key:, peer_id:, timestamp:, body:)` returns the base64 signature. `Crypto.verify(public_key:, peer_id:, timestamp:, body:, signature:, skew_seconds: 300)` returns true/false. Signature input is `"#{peer_id}\n#{timestamp}\n#{sha256_hex(body)}"`.

**Step 1: Write failing tests**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::CryptoTest < Minitest::Test
  def setup
    @signing_key = Ed25519::SigningKey.generate
    @public_key  = @signing_key.verify_key.to_bytes
    @private_key = @signing_key.to_bytes
    @peer_id     = "peer-x"
    @ts          = Time.now.utc.iso8601
    @body        = '{"hello":"world"}'
  end

  def test_sign_and_verify_round_trip
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)
    assert Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: @ts, body: @body, signature: sig)
  end

  def test_verify_fails_on_mutated_body
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)
    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: @ts, body: "tampered", signature: sig)
  end

  def test_verify_fails_on_mutated_peer_id
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: @ts, body: @body)
    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: "other", timestamp: @ts, body: @body, signature: sig)
  end

  def test_verify_rejects_skewed_timestamp
    old = (Time.now.utc - 3600).iso8601
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: old, body: @body)
    refute Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: old, body: @body, signature: sig)
  end

  def test_verify_accepts_within_skew
    old = (Time.now.utc - 60).iso8601
    sig = Recollect::Sync::Crypto.sign(private_key: @private_key, peer_id: @peer_id, timestamp: old, body: @body)
    assert Recollect::Sync::Crypto.verify(public_key: @public_key, peer_id: @peer_id, timestamp: old, body: @body, signature: sig)
  end
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/sync/crypto_test.rb
```

**Step 3: Implement**

```ruby
# frozen_string_literal: true
require "ed25519"
require "base64"
require "digest"
require "time"

module Recollect
  module Sync
    module Crypto
      DEFAULT_SKEW_SECONDS = 300

      module_function

      def sign(private_key:, peer_id:, timestamp:, body:)
        sk = Ed25519::SigningKey.new(private_key)
        Base64.strict_encode64(sk.sign(payload(peer_id, timestamp, body)))
      end

      def verify(public_key:, peer_id:, timestamp:, body:, signature:, skew_seconds: DEFAULT_SKEW_SECONDS)
        return false unless within_skew?(timestamp, skew_seconds)
        vk = Ed25519::VerifyKey.new(public_key)
        vk.verify(Base64.strict_decode64(signature), payload(peer_id, timestamp, body))
      rescue Ed25519::VerifyError, ArgumentError, ArgumentError
        false
      end

      def payload(peer_id, timestamp, body)
        "#{peer_id}\n#{timestamp}\n#{Digest::SHA256.hexdigest(body || "")}"
      end

      def within_skew?(timestamp, skew_seconds)
        ts = Time.iso8601(timestamp.to_s)
        (Time.now.utc - ts).abs <= skew_seconds
      rescue ArgumentError
        false
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/crypto_test.rb
```

**Step 5: Commit**

```bash
git add lib/recollect/sync/crypto.rb test/recollect/sync/crypto_test.rb
git commit -m "feat(sync): Ed25519 request signing and verification helpers"
```

---

### Task 11: Endpoint discovery (RECOLLECT_PUBLIC_URL → tailscale → LAN IP → error)

**Files:**
- Create: `lib/recollect/sync/endpoint.rb`
- Create: `test/recollect/sync/endpoint_test.rb`

**Context:** Returns the URL another peer should use to reach us. Resolution order: env var, `tailscale status --json` MagicDNS name, first non-loopback non-link-local IPv4 from `Socket.ip_address_list`, raise. Tests use stubs for the system calls.

**Step 1: Write failing tests**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::EndpointTest < Minitest::Test
  def setup
    @port = 7326
  end

  def test_env_var_wins
    ENV["RECOLLECT_PUBLIC_URL"] = "http://override.example:9000"
    assert_equal "http://override.example:9000", Recollect::Sync::Endpoint.discover(port: @port)
  ensure
    ENV.delete("RECOLLECT_PUBLIC_URL")
  end

  def test_falls_back_to_tailscale_when_env_unset
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, "laptop.tailnet.ts.net") do
      assert_equal "http://laptop.tailnet.ts.net:#{@port}", Recollect::Sync::Endpoint.discover(port: @port)
    end
  end

  def test_falls_back_to_lan_ip_when_no_tailscale
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, nil) do
      Recollect::Sync::Endpoint.stub(:lan_ipv4, "192.168.1.42") do
        assert_equal "http://192.168.1.42:#{@port}", Recollect::Sync::Endpoint.discover(port: @port)
      end
    end
  end

  def test_raises_when_nothing_resolves
    Recollect::Sync::Endpoint.stub(:tailscale_dns_name, nil) do
      Recollect::Sync::Endpoint.stub(:lan_ipv4, nil) do
        err = assert_raises(Recollect::Sync::Endpoint::DiscoveryError) do
          Recollect::Sync::Endpoint.discover(port: @port)
        end
        assert_match(/RECOLLECT_PUBLIC_URL/, err.message)
      end
    end
  end
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/sync/endpoint_test.rb
```

**Step 3: Implement**

```ruby
# frozen_string_literal: true
require "json"
require "open3"
require "socket"

module Recollect
  module Sync
    module Endpoint
      class DiscoveryError < StandardError; end

      module_function

      def discover(port:)
        if (env = ENV["RECOLLECT_PUBLIC_URL"]) && !env.empty?
          return env
        end
        if (name = tailscale_dns_name)
          return "http://#{name}:#{port}"
        end
        if (ip = lan_ipv4)
          return "http://#{ip}:#{port}"
        end
        raise DiscoveryError, "Could not auto-detect a reachable URL. Set RECOLLECT_PUBLIC_URL or pass --endpoint."
      end

      def tailscale_dns_name
        out, status = Open3.capture2("tailscale", "status", "--json")
        return nil unless status.success?
        data = JSON.parse(out)
        self_node = data["Self"] or return nil
        dns = self_node["DNSName"]
        dns&.chomp(".") if dns && !dns.empty?
      rescue Errno::ENOENT, JSON::ParserError
        nil
      end

      def lan_ipv4
        Socket.ip_address_list.find do |a|
          a.ipv4? && !a.ipv4_loopback? && !a.ipv4_linklocal?
        end&.ip_address
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/endpoint_test.rb
```

**Step 5: Commit**

```bash
git add lib/recollect/sync/endpoint.rb test/recollect/sync/endpoint_test.rb
git commit -m "feat(sync): endpoint discovery with env > tailscale > LAN IP fallback"
```

---

## Pairing

### Task 12: Pairing code generation and validation

**Files:**
- Create: `lib/recollect/sync/pairing_codes.rb`
- Create: `test/recollect/sync/pairing_codes_test.rb`

**Context:** Generate codes (8 chars, uppercase alphanumeric, hyphenated `XXXX-XXXX`), 5-min TTL, single-use. Persist in `pairing_codes` table. Validation: exists, not expired, not used. Mark used atomically (UPDATE ... WHERE used_at IS NULL).

**Step 1: Write failing tests**

```ruby
# frozen_string_literal: true
require "test_helper"

class Recollect::Sync::PairingCodesTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @codes = Recollect::Sync::PairingCodes.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def test_generate_returns_code_and_expiry
    result = @codes.generate(ttl_seconds: 300)
    assert_match(/\A[A-Z0-9]{4}-[A-Z0-9]{4}\z/, result[:code])
    assert_kind_of Time, result[:expires_at]
    assert_in_delta 300, (result[:expires_at] - Time.now.utc), 5
  end

  def test_consume_returns_true_for_valid_code
    code = @codes.generate[:code]
    assert_equal true, @codes.consume(code, used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_unknown_code
    assert_equal false, @codes.consume("ZZZZ-ZZZZ", used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_expired_code
    code = @codes.generate(ttl_seconds: -1)[:code]  # already expired
    assert_equal false, @codes.consume(code, used_by_peer: "peer-x")
  end

  def test_consume_returns_false_for_already_used_code
    code = @codes.generate[:code]
    assert_equal true,  @codes.consume(code, used_by_peer: "peer-x")
    assert_equal false, @codes.consume(code, used_by_peer: "peer-y")
  end
end
```

**Step 2: Run, expect FAIL**

```bash
bundle exec ruby -Itest test/recollect/sync/pairing_codes_test.rb
```

**Step 3: Implement**

```ruby
# frozen_string_literal: true
require "securerandom"
require "time"

module Recollect
  module Sync
    class PairingCodes
      ALPHABET = ("A".."Z").to_a + ("0".."9").to_a

      def initialize(store)
        @store = store
      end

      def generate(ttl_seconds: 300)
        code = "#{rand_block}-#{rand_block}"
        now  = Time.now.utc
        exp  = now + ttl_seconds
        db.execute(
          "INSERT INTO pairing_codes (code, created_at, expires_at) VALUES (?, ?, ?)",
          [code, now.iso8601, exp.iso8601]
        )
        { code: code, expires_at: exp }
      end

      def consume(code, used_by_peer:)
        now = Time.now.utc.iso8601
        db.execute(
          "UPDATE pairing_codes SET used_at = ?, used_by_peer = ? WHERE code = ? AND used_at IS NULL AND expires_at > ?",
          [now, used_by_peer, code, now]
        )
        db.changes.positive?
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end

      def rand_block
        Array.new(4) { ALPHABET.sample(random: SecureRandom) }.join
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

```bash
bundle exec ruby -Itest test/recollect/sync/pairing_codes_test.rb
```

**Step 5: Commit**

```bash
git add lib/recollect/sync/pairing_codes.rb test/recollect/sync/pairing_codes_test.rb
git commit -m "feat(sync): pairing code generation and single-use consumption"
```

---

### Task 13: Add `Sync::Peers` registry (CRUD over `known_peers` and `peer_db_subscriptions`)

**Files:**
- Create: `lib/recollect/sync/peers.rb`
- Create: `test/recollect/sync/peers_test.rb`

**Step 1: Write failing tests**

```ruby
require "test_helper"

class Recollect::Sync::PeersTest < Recollect::TestCase
  def setup
    super
    @store = Recollect::Sync::Store.new(Recollect.config.sync_db_path)
    @peers = Recollect::Sync::Peers.new(@store)
  end

  def teardown
    @store.close
    super
  end

  def peer_attrs(id)
    { peer_id: id, display_name: "n-#{id}", public_key: ("\x00" * 32).b, endpoint: "http://#{id}:7326" }
  end

  def test_add_and_list
    @peers.add(**peer_attrs("p1"))
    @peers.add(**peer_attrs("p2"))
    list = @peers.list
    assert_equal 2, list.size
    assert_equal %w[p1 p2].sort, list.map { |p| p[:peer_id] }.sort
  end

  def test_block
    @peers.add(**peer_attrs("p1"))
    @peers.block("p1")
    p1 = @peers.find("p1")
    assert_equal "blocked", p1[:status]
  end

  def test_subscribe_and_subscriptions
    @peers.add(**peer_attrs("p1"))
    @peers.subscribe("p1", "global")
    @peers.subscribe("p1", "personal-finance")
    assert_equal %w[global personal-finance].sort, @peers.subscriptions("p1").sort
  end

  def test_unsubscribe
    @peers.add(**peer_attrs("p1"))
    @peers.subscribe("p1", "global")
    @peers.unsubscribe("p1", "global")
    assert_empty @peers.subscriptions("p1")
  end

  def test_default_subscription_on_add
    @peers.add(**peer_attrs("p1"), default_subscription: "global")
    assert_equal ["global"], @peers.subscriptions("p1")
  end
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
# frozen_string_literal: true
require "time"

module Recollect
  module Sync
    class Peers
      def initialize(store)
        @store = store
      end

      def add(peer_id:, display_name:, public_key:, endpoint:, default_subscription: nil)
        db.execute(
          "INSERT INTO known_peers (peer_id, display_name, public_key, endpoint, status, trusted_at) VALUES (?, ?, ?, ?, 'trusted', ?) ON CONFLICT(peer_id) DO UPDATE SET display_name=excluded.display_name, public_key=excluded.public_key, endpoint=excluded.endpoint, status='trusted'",
          [peer_id, display_name, SQLite3::Blob.new(public_key), endpoint, Time.now.utc.iso8601]
        )
        subscribe(peer_id, default_subscription) if default_subscription
      end

      def find(peer_id)
        row = db.get_first_row("SELECT * FROM known_peers WHERE peer_id = ?", peer_id)
        row && symbolize(row)
      end

      def list
        db.execute("SELECT * FROM known_peers ORDER BY peer_id").map { |r| symbolize(r) }
      end

      def block(peer_id)
        db.execute("UPDATE known_peers SET status = 'blocked' WHERE peer_id = ?", peer_id)
      end

      def subscribe(peer_id, db_name)
        db.execute("INSERT OR IGNORE INTO peer_db_subscriptions (peer_id, db_name) VALUES (?, ?)", [peer_id, db_name])
      end

      def unsubscribe(peer_id, db_name)
        db.execute("DELETE FROM peer_db_subscriptions WHERE peer_id = ? AND db_name = ?", [peer_id, db_name])
      end

      def subscriptions(peer_id)
        db.execute("SELECT db_name FROM peer_db_subscriptions WHERE peer_id = ? ORDER BY db_name", peer_id).map { |r| r["db_name"] }
      end

      private

      def db
        @store.instance_variable_get(:@db)
      end

      def symbolize(row)
        { peer_id: row["peer_id"], display_name: row["display_name"], public_key: row["public_key"],
          endpoint: row["endpoint"], status: row["status"], trusted_at: row["trusted_at"],
          last_seen_at: row["last_seen_at"], last_sync_at: row["last_sync_at"], last_sync_error: row["last_sync_error"] }
      end
    end
  end
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/sync/peers.rb test/recollect/sync/peers_test.rb
git commit -m "feat(sync): Sync::Peers registry with subscriptions"
```

---

### Task 14: `POST /pairing/create` endpoint (local-only)

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Context:** Generates a pairing code, returns `{ code, expires_at, endpoint }`. Rejects non-loopback callers with 403. Endpoint discovered via `Sync::Endpoint.discover(port: config.port)`.

**Step 1: Write failing tests**

```ruby
def test_pairing_create_returns_code_for_loopback
  post "/pairing/create"
  assert_equal 200, last_response.status
  body = JSON.parse(last_response.body)
  assert_match(/\A[A-Z0-9]{4}-[A-Z0-9]{4}\z/, body["code"])
  refute_nil body["expires_at"]
  refute_nil body["endpoint"]
end

def test_pairing_create_rejects_non_loopback
  header "X-Forwarded-For", "8.8.8.8"
  # Simulate non-loopback by overriding REMOTE_ADDR
  post "/pairing/create", {}, "REMOTE_ADDR" => "8.8.8.8"
  assert_equal 403, last_response.status
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

In `lib/recollect/http_server.rb`:

```ruby
helpers do
  def loopback_request?
    addr = request.env["REMOTE_ADDR"]
    %w[127.0.0.1 ::1].include?(addr)
  end

  def pairing_codes
    @pairing_codes ||= Recollect::Sync::PairingCodes.new(self.class.sync_store)
  end
end

post "/pairing/create" do
  halt 403, { error: "local-only endpoint" }.to_json unless loopback_request?
  result = pairing_codes.generate
  endpoint = Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
  content_type :json
  { code: result[:code], expires_at: result[:expires_at].iso8601, endpoint: endpoint }.to_json
end
```

(If `Endpoint.discover` raises, let Sinatra return a 500 — test it separately if you want.)

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): POST /pairing/create (local-only) returns code + endpoint"
```

---

### Task 15: `POST /pairing/join` endpoint

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Context:** Receives `{ code, peer_id, display_name, public_key (base64), endpoint }`. Validates+consumes code. Inserts joiner into `known_peers` (status=trusted) with default subscription `'global'`. Returns local peer's identity (peer_id, display_name, public_key base64, endpoint). No signature on this endpoint — the code itself is the auth.

**Step 1: Write failing tests**

```ruby
def test_pairing_join_with_valid_code_trusts_peer
  # Create a code first
  post "/pairing/create"
  code = JSON.parse(last_response.body)["code"]

  joiner_pub = ("\x42" * 32).b
  payload = {
    code: code,
    peer_id: "joiner-x",
    display_name: "Joiner",
    public_key: Base64.strict_encode64(joiner_pub),
    endpoint: "http://joiner:7326"
  }
  post "/pairing/join", payload.to_json, "CONTENT_TYPE" => "application/json"
  assert_equal 200, last_response.status

  body = JSON.parse(last_response.body)
  assert_equal Recollect::HTTPServer.local_identity.peer_id, body["peer_id"]
  refute_nil body["public_key"]
  refute_nil body["endpoint"]

  # Joiner is now in known_peers with global subscription
  peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
  joiner = peers.find("joiner-x")
  refute_nil joiner
  assert_equal "trusted", joiner[:status]
  assert_equal ["global"], peers.subscriptions("joiner-x")
end

def test_pairing_join_with_invalid_code_rejected
  payload = {
    code: "ZZZZ-ZZZZ", peer_id: "x", display_name: "x",
    public_key: Base64.strict_encode64("\x00" * 32), endpoint: "http://x:7326"
  }
  post "/pairing/join", payload.to_json, "CONTENT_TYPE" => "application/json"
  assert_equal 401, last_response.status
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
post "/pairing/join" do
  body = JSON.parse(request.body.read)
  code = body["code"]
  joiner_id = body["peer_id"]

  unless pairing_codes.consume(code, used_by_peer: joiner_id)
    halt 401, { error: "invalid or expired code" }.to_json
  end

  peers = Recollect::Sync::Peers.new(self.class.sync_store)
  peers.add(
    peer_id: joiner_id,
    display_name: body["display_name"],
    public_key: Base64.strict_decode64(body["public_key"]),
    endpoint: body["endpoint"],
    default_subscription: "global"
  )

  identity = self.class.local_identity
  content_type :json
  {
    peer_id: identity.peer_id,
    display_name: identity.display_name,
    public_key: Base64.strict_encode64(identity.public_key),
    endpoint: Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
  }.to_json
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): POST /pairing/join trusts joiner and returns local identity"
```

---

## CLI

### Task 16: REST endpoints for CLI to drive (`/api/sync/...`)

**Files:**
- Modify: `lib/recollect/http_server.rb`
- Test: `test/recollect/http_server_test.rb`

**Context:** The CLI is HTTP-based and never touches the SyncStore directly. Add small JSON endpoints under `/api/sync/` so the CLI can read identity, list/manage peers, and trigger pair-create. These are local-only (loopback check).

**Step 1: Write failing tests**

```ruby
def test_api_sync_identity
  get "/api/sync/identity"
  assert_equal 200, last_response.status
  body = JSON.parse(last_response.body)
  assert_equal Recollect::HTTPServer.local_identity.peer_id, body["peer_id"]
  refute_nil body["display_name"]
  refute_nil body["public_key_fingerprint"]
end

def test_api_sync_peers_list_empty_initially
  get "/api/sync/peers"
  assert_equal 200, last_response.status
  assert_equal [], JSON.parse(last_response.body)
end

def test_api_sync_peers_remove_blocks
  peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
  peers.add(peer_id: "p1", display_name: "P1", public_key: ("\x00" * 32).b, endpoint: "http://p1:7326")
  delete "/api/sync/peers/p1"
  assert_equal 200, last_response.status
  assert_equal "blocked", peers.find("p1")[:status]
end

def test_api_sync_peers_subscriptions_add_remove
  peers = Recollect::Sync::Peers.new(Recollect::HTTPServer.sync_store)
  peers.add(peer_id: "p1", display_name: "P1", public_key: ("\x00" * 32).b, endpoint: "http://p1:7326")

  post "/api/sync/peers/p1/subscriptions", { db_name: "personal-finance" }.to_json, "CONTENT_TYPE" => "application/json"
  assert_equal 200, last_response.status
  assert_includes peers.subscriptions("p1"), "personal-finance"

  delete "/api/sync/peers/p1/subscriptions/personal-finance"
  assert_equal 200, last_response.status
  refute_includes peers.subscriptions("p1"), "personal-finance"
end
```

**Step 2: Run, expect FAIL**

**Step 3: Implement**

```ruby
helpers do
  def require_loopback!
    halt 403, { error: "local-only endpoint" }.to_json unless loopback_request?
  end

  def peers_registry
    @peers_registry ||= Recollect::Sync::Peers.new(self.class.sync_store)
  end
end

get "/api/sync/identity" do
  require_loopback!
  content_type :json
  identity = self.class.local_identity
  fingerprint = Digest::SHA256.hexdigest(identity.public_key)[0, 16]
  { peer_id: identity.peer_id, display_name: identity.display_name, public_key_fingerprint: fingerprint }.to_json
end

get "/api/sync/peers" do
  require_loopback!
  content_type :json
  peers_registry.list.map do |p|
    p.merge(subscriptions: peers_registry.subscriptions(p[:peer_id])).tap { |h| h.delete(:public_key) }
  end.to_json
end

delete "/api/sync/peers/:peer_id" do
  require_loopback!
  peers_registry.block(params[:peer_id])
  content_type :json
  { ok: true }.to_json
end

post "/api/sync/peers/:peer_id/subscriptions" do
  require_loopback!
  body = JSON.parse(request.body.read)
  peers_registry.subscribe(params[:peer_id], body["db_name"])
  content_type :json
  { ok: true }.to_json
end

delete "/api/sync/peers/:peer_id/subscriptions/:db_name" do
  require_loopback!
  peers_registry.unsubscribe(params[:peer_id], params[:db_name])
  content_type :json
  { ok: true }.to_json
end
```

**Step 4: Run, expect PASS**

**Step 5: Commit**

```bash
git add lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(sync): /api/sync REST endpoints for CLI (identity + peers CRUD)"
```

---

### Task 17: CLI `recollect identity`

**Files:**
- Modify: `bin/recollect`

**Step 1: Write a smoke test (optional but recommended)**

If there's an existing CLI test file, add to it; if not, skip this step and rely on manual smoke. Either way:

```bash
# Manual smoke: with server running:
./bin/recollect identity
# Expected: prints peer_id, display_name, public_key_fingerprint
```

**Step 2: Implement**

In `bin/recollect`, inside `class CLI < Thor`:

```ruby
desc "identity", "Show the local sync identity"
def identity
  response = get_request("/api/sync/identity")
  if response.code == "200"
    data = JSON.parse(response.body)
    say "peer_id:     #{@pastel.cyan(data["peer_id"])}"
    say "name:        #{data["display_name"]}"
    say "fingerprint: #{data["public_key_fingerprint"]}"
  else
    say @pastel.red("Error: #{response.code} #{response.message}")
  end
rescue => e
  say @pastel.red("Error: #{e.message}")
end
```

(`get_request` is the existing helper used by other commands; if it doesn't exist, add a thin wrapper or inline the `Net::HTTP.get_response`.)

**Step 3: Manual smoke test**

```bash
./bin/server &
sleep 1
./bin/recollect identity
# Expected: three lines as above
kill %1
```

**Step 4: Commit**

```bash
git add bin/recollect
git commit -m "feat(cli): recollect identity shows local sync identity"
```

---

### Task 18: CLI `recollect pair`

**Files:**
- Modify: `bin/recollect`

**Step 1: Implement**

```ruby
desc "pair", "Generate a pairing code for another machine to use"
def pair
  response = post("/pairing/create", {})
  if response.code == "200"
    data = JSON.parse(response.body)
    say @pastel.bold("Pairing code: #{@pastel.cyan(data["code"])}")
    say "Endpoint:     #{data["endpoint"]}"
    say "Expires:      #{data["expires_at"]} (5 minutes)"
    say ""
    say "On the other machine, run:"
    say "  recollect join #{data["code"]} --endpoint #{data["endpoint"]}"
  else
    say @pastel.red("Error: #{response.code} #{response.message}")
  end
rescue => e
  say @pastel.red("Error: #{e.message}")
end
```

**Step 2: Manual smoke**

```bash
./bin/server &
sleep 1
./bin/recollect pair
# Expected: code, endpoint, expiry, join instructions
kill %1
```

**Step 3: Commit**

```bash
git add bin/recollect
git commit -m "feat(cli): recollect pair generates and displays pairing code"
```

---

### Task 19: CLI `recollect join`

**Files:**
- Modify: `bin/recollect`

**Context:** Joiner doesn't go through its own HTTP server; it makes a direct outbound HTTP call to the other peer. So this command needs to read the local identity (to send) and POST to `<peer_endpoint>/pairing/join`. Then on success, store the response peer in its OWN sync store via `/api/sync/peers/internal/add` (or directly via a helper REST endpoint).

The simplest pattern: add a `POST /api/sync/peers/join` endpoint on the joining side that takes `{ code, endpoint }`, looks up the local identity, makes the outbound call, and stores the response peer locally. CLI just calls this endpoint.

**Step 1: Add the server-side endpoint** in `lib/recollect/http_server.rb`:

```ruby
post "/api/sync/peers/join" do
  require_loopback!
  body = JSON.parse(request.body.read)
  code = body["code"]
  peer_endpoint = body["endpoint"]
  identity = self.class.local_identity

  payload = {
    code: code,
    peer_id: identity.peer_id,
    display_name: identity.display_name,
    public_key: Base64.strict_encode64(identity.public_key),
    endpoint: Recollect::Sync::Endpoint.discover(port: Recollect.config.port)
  }

  response = Faraday.post("#{peer_endpoint}/pairing/join") do |req|
    req.headers["Content-Type"] = "application/json"
    req.body = payload.to_json
  end

  unless response.success?
    halt response.status, response.body
  end

  remote = JSON.parse(response.body)
  peers_registry.add(
    peer_id: remote["peer_id"],
    display_name: remote["display_name"],
    public_key: Base64.strict_decode64(remote["public_key"]),
    endpoint: remote["endpoint"],
    default_subscription: "global"
  )

  content_type :json
  remote.merge("status" => "trusted").to_json
end
```

Add `require "faraday"` near the other requires.

**Step 2: Test the endpoint**

```ruby
def test_api_sync_peers_join_round_trip
  # Stand up two HTTPServer instances: one as A (initiator), one as B (joiner)
  # via Rack::Test. Generate code on A, call /api/sync/peers/join on B with that code.
  # ...This may be too involved for unit-level. Cover with manual smoke and a Phase-2 integration test.
end
```

(For Phase 1 it's acceptable to cover this end-to-end manually; the constituent pieces are unit-tested.)

**Step 3: Implement CLI command**

```ruby
desc "join CODE", "Join an existing peer using a pairing code"
option :endpoint, required: true, desc: "Peer's URL (e.g. http://laptop.tailnet.ts.net:7326)"
def join(code)
  response = post("/api/sync/peers/join", { code: code, endpoint: options[:endpoint] })
  if response.code == "200"
    data = JSON.parse(response.body)
    say @pastel.green("Paired with #{data["display_name"]} (#{data["peer_id"]})")
  else
    say @pastel.red("Error: #{response.code} #{response.message}")
    say response.body
  end
rescue => e
  say @pastel.red("Error: #{e.message}")
end
```

**Step 4: Manual smoke**

Run two instances on different ports / data dirs, pair via `recollect pair` on A, then `recollect join CODE --endpoint http://A:7326` on B. Confirm both `recollect peers` show each other.

**Step 5: Commit**

```bash
git add bin/recollect lib/recollect/http_server.rb test/recollect/http_server_test.rb
git commit -m "feat(cli): recollect join performs full pairing handshake"
```

---

### Task 20: CLI `recollect peers` (list / remove / add-db / remove-db)

**Files:**
- Modify: `bin/recollect`

**Step 1: Implement subcommand**

```ruby
class Peers < Thor
  def initialize(*)
    super
    @pastel = Pastel.new
    @base_url = ENV.fetch("RECOLLECT_URL", Recollect.config.url)
  end

  desc "list", "List trusted peers"
  def list
    response = Net::HTTP.get_response(URI("#{@base_url}/api/sync/peers"))
    if response.code == "200"
      data = JSON.parse(response.body)
      if data.empty?
        say "No peers."
      else
        table = TTY::Table.new(["peer_id", "name", "endpoint", "status", "last_seen", "subscriptions"], data.map { |p|
          [p["peer_id"], p["display_name"], p["endpoint"], p["status"], p["last_seen_at"] || "-", p["subscriptions"].join(",")]
        })
        puts table.render(:unicode)
      end
    end
  end

  desc "remove PEER_ID", "Block a peer (kept in DB for signature history)"
  def remove(peer_id)
    uri = URI("#{@base_url}/api/sync/peers/#{peer_id}")
    response = Net::HTTP.new(uri.host, uri.port).delete(uri.path)
    say response.code == "200" ? @pastel.green("Blocked #{peer_id}") : @pastel.red("Error: #{response.code}")
  end

  desc "add-db PEER_ID DB_NAME", "Subscribe a peer to one of your DBs"
  def add_db(peer_id, db_name)
    uri = URI("#{@base_url}/api/sync/peers/#{peer_id}/subscriptions")
    response = Net::HTTP.start(uri.host, uri.port) do |http|
      req = Net::HTTP::Post.new(uri.path, "Content-Type" => "application/json")
      req.body = { db_name: db_name }.to_json
      http.request(req)
    end
    say response.code == "200" ? @pastel.green("Subscribed #{peer_id} to #{db_name}") : @pastel.red("Error: #{response.code}")
  end

  desc "remove-db PEER_ID DB_NAME", "Unsubscribe a peer from one of your DBs"
  def remove_db(peer_id, db_name)
    uri = URI("#{@base_url}/api/sync/peers/#{peer_id}/subscriptions/#{db_name}")
    response = Net::HTTP.new(uri.host, uri.port).delete(uri.path)
    say response.code == "200" ? @pastel.green("Unsubscribed #{peer_id} from #{db_name}") : @pastel.red("Error: #{response.code}")
  end
end

# Inside CLI < Thor:
desc "peers SUBCOMMAND", "Manage sync peers"
subcommand "peers", Peers
```

**Step 2: Manual smoke**

```bash
./bin/recollect peers list
./bin/recollect peers add-db <peer_id> personal-finance
./bin/recollect peers remove <peer_id>
```

**Step 3: Commit**

```bash
git add bin/recollect
git commit -m "feat(cli): recollect peers list/remove/add-db/remove-db"
```

---

## Final Verification

### Task 21: Run rubocop, tests, and coverage

**Step 1: Lint**

```bash
bundle exec rake rubocop
```

Fix any offenses. Many will be minor (string quoting, line length). Run with `-A` if safe-autocorrect helps.

**Step 2: Full test suite**

```bash
bundle exec rake test
```

All green.

**Step 3: Coverage**

```bash
bundle exec rake coverage
```

Must not regress below the existing 86.97%. If sync code introduces gaps, add focused tests for the missing branches.

**Step 4: End-to-end smoke**

In two terminals, with two distinct `RECOLLECT_DATA_DIR`s and two distinct ports:

```bash
# Terminal A
RECOLLECT_DATA_DIR=/tmp/rec-a RECOLLECT_PORT=7326 RECOLLECT_PUBLIC_URL=http://127.0.0.1:7326 ./bin/server

# Terminal B
RECOLLECT_DATA_DIR=/tmp/rec-b RECOLLECT_PORT=7327 RECOLLECT_PUBLIC_URL=http://127.0.0.1:7327 ./bin/server

# Terminal A (CLI)
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect identity
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect pair
# Copy code; in another terminal:
RECOLLECT_URL=http://127.0.0.1:7327 ./bin/recollect join <CODE> --endpoint http://127.0.0.1:7326
RECOLLECT_URL=http://127.0.0.1:7326 ./bin/recollect peers list   # should show B
RECOLLECT_URL=http://127.0.0.1:7327 ./bin/recollect peers list   # should show A
```

Both peers should see each other, status `trusted`, subscription `global`.

**Step 5: Final commit (if any pending work)**

```bash
git status
# If clean: nothing to commit. Otherwise commit any rubocop fixups:
git commit -m "chore: rubocop fixes for sync phase 1"
```

---

## Done — what's next

Phase 1 is complete: schema migrated, identity generated, tombstones working, pairing works end-to-end, CLI surface in place. **No records flow yet** — that's Phase 2.

Before starting Phase 2, decide the chunk-handling question (open question at top of this doc) and write the Phase 2 plan.
