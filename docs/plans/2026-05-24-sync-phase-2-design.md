# Recollect Sync — Phase 2 (Ambient Sync) Design

**Date:** 2026-05-24
**Branch:** `sync/phase-2` (to be cut from `sync/phase-1` once merged, or from `master` if merged first)
**Builds on:** Phase 1 ([2026-05-02-sync-phase-1-foundation.md](2026-05-02-sync-phase-1-foundation.md)) — schema, identity, crypto, pairing, CLI scaffolding.
**Master design:** [2026-05-02-sync-design.md](2026-05-02-sync-design.md) — protocol shape, error model, watermark semantics. **This supplement only addresses what the master left open or under-specified.**

## Goal

Two paired Recollect instances stay converged ambiently: records created on A appear on B within seconds (push-on-write), and any drift caused by network errors or downtime is reconciled by a heartbeat loop. No manual intervention required after pairing.

## Resolved open questions

### Chunk handling

`_chunk` rows are **never synced**. Chunks are per-peer derived state: they only exist to drive vector retrieval, and the embedding model is a local concern. After a non-chunk record is upserted by sync, if vectors are enabled locally, the receiver enqueues `(local_id, content, project)` on the existing `EmbeddingWorker`, which re-chunks via `MarkdownChunker` and re-embeds. Receivers with vectors disabled simply hold the parent row; vector search returns nothing for it.

Concretely:
- Push/pull SQL filters with `WHERE memory_type != '_chunk'`.
- Embedding bytes are **never** included in sync payloads.
- The existing `EmbeddingWorker.enqueue` API is the receive-side hook (no new worker class).
- The `metadata.parent_id` ↔ `parent_global_id` translation problem is sidestepped entirely.

This overrides the master doc's line "Embedding BLOB included in record payloads (base64)." That bullet is dropped.

### Test topology

Two-peer integration tests run **in-process**, not as two real Puma servers. The outbound Faraday client used by `Sync::Engine` and `Sync::PushQueue` is constructed through a small factory that returns either a network adapter (production) or a Rack adapter pointing at the other `HTTPServer` instance (tests). This keeps tests fast and deterministic. The push worker and engine threads still run; only the I/O substrate is swapped.

Single-peer tests continue to use `Rack::Test` as today.

### Re-embed path

Reuse `DatabaseManager#enqueue_for_embedding` (new convenience method around the existing `EmbeddingWorker`). No `Sync::Reembedder`. The same `SizedQueue` that protects local writes protects sync receives. If the queue fills under an initial-sync burst, the producer (sync receive) blocks — acceptable because sync is a background activity.

## What's new in Phase 2 (full inventory)

### Endpoints

| Endpoint | Auth | Purpose |
|---|---|---|
| `GET /sync/manifest?db=<db_name>` | signature | Return our `peer_watermarks` for this DB plus our self-watermark. |
| `POST /sync/pull?db=<db_name>` | signature | `{since, limit}` → records newer than `since` per origin. Pagination via `limit` + caller re-issues. |
| `POST /sync/push?db=<db_name>` | signature | `{records}` → upsert, advance watermarks, enqueue re-embed. |

Signature middleware: Sinatra `before '/sync/*'` filter calls `Sync::SignatureVerifier.verify(request)` → on success, sets `env['recollect.peer']`; on failure, halts 401 (bad sig / skew) or 403 (peer is `blocked` or unknown).

### Background workers

| Class | Purpose |
|---|---|
| `Sync::Engine` | One thread. Pull-on-startup pass, then heartbeat loop every `RECOLLECT_SYNC_HEARTBEAT_SECONDS` (default 300). Per (peer, subscribed db): manifest → pull (paginate to drain) → push (records peer is missing). |
| `Sync::PushQueue` | `SizedQueue.new(1000)` + one worker thread. `MemoriesService#store` / `#delete` enqueue `(global_id, db_name)` after commit. Worker loads record, fans out `POST /sync/push` to subscribed peers. |
| `Sync::Client` | Thin wrapper around Faraday that signs outbound requests. Handles JSON, base64-decodes nothing (chunks aren't synced; embedding bytes aren't sent). |

### Sender-side outbound subscription filter

`peer_db_subscriptions(peer_id, db_name)` says "I share my DB `<db_name>` with peer `<peer_id>`." On `/sync/pull`, the responder consults the caller's row in this table to decide whether to return any rows for the requested `db`. If the caller is not subscribed, return `{records: []}` (not 403 — subscription is a sender-side policy, not authorization, and may be changed later without re-pairing). Same filter applied at `Sync::PushQueue` fan-out.

### MemoriesService push hooks

Two new hook points:

```ruby
# In MemoriesService#store, after db_manager commits:
HTTPServer.push_queue.enqueue(global_id: record.global_id, db_name: project)

# In MemoriesService#delete, after tombstone commits:
HTTPServer.push_queue.enqueue(global_id: record.global_id, db_name: project)
```

`Database#store` needs to return `global_id` alongside `id` (currently returns only `id`). Smallest change: return `{id:, global_id:}` or add a `last_global_id` accessor. The plan will pick one and apply it once.

### Upsert semantics

Receiver-side insert is exactly the SQL the master doc specifies:

```sql
INSERT INTO memories (global_id, origin_peer, created_at, content, memory_type, tags, metadata, deleted_at, deleted_by_peer)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(global_id) DO UPDATE SET
  deleted_at      = COALESCE(memories.deleted_at, excluded.deleted_at),
  deleted_by_peer = CASE
    WHEN memories.deleted_at IS NULL AND excluded.deleted_at IS NOT NULL
    THEN excluded.deleted_by_peer
    ELSE memories.deleted_by_peer
  END
```

First-write wins for content. First-tombstone wins for deletion. Late content for a tombstoned `global_id` is a no-op on content. Tombstone-before-content converges to deleted.

Wrapped in `BEGIN IMMEDIATE` per push batch, with watermark advance in the same transaction.

### Watermark advance

After applying a push batch (or each pull page), per origin observed:

```sql
INSERT INTO peer_watermarks (peer_id, db_name, latest_created_at)
VALUES (?, ?, ?)
ON CONFLICT(peer_id, db_name) DO UPDATE SET
  latest_created_at = MAX(peer_watermarks.latest_created_at, excluded.latest_created_at);
```

`peer_id` here is the **origin_peer of the record**, not the sender.

### CLI: `recollect sync`

```
recollect sync [--peer PEER_ID] [--db DB_NAME]
```

POSTs `/api/sync/sync` (new local-only endpoint) which kicks the engine into a one-shot reconciliation pass. Prints per-peer per-db: records pulled, records pushed, watermarks before/after. Useful during initial rollout; quietly omitted from `recollect help` (`hide: true`).

### `last_sync_at` / `last_sync_error`

Already in the Phase 1 schema (`known_peers` columns). Engine and push worker update these. CLI `peers list` already displays them (Phase 1 task 20).

## What's NOT in Phase 2 (still deferred)

- Re-embed pass when embedding model changes (manual procedure).
- Vacuum / permanent deletion of old tombstones.
- HTTPS / cert management (Tailscale provides transport encryption).
- Encrypted-at-rest private key (file mode 0600 only).
- Web UI for peer management.
- Push fan-out batching beyond one record per push call (heartbeat handles backlog).

## Environment variables (new in Phase 2)

| Variable | Default | Description |
|---|---|---|
| `RECOLLECT_SYNC_HEARTBEAT_SECONDS` | `300` | Heartbeat interval for `Sync::Engine`. Set to `0` to disable. |
| `RECOLLECT_SYNC_PUSH_QUEUE_SIZE` | `1000` | `SizedQueue` capacity for `Sync::PushQueue`. |
| `RECOLLECT_SYNC_DISABLE` | (unset) | If set to `1`, neither engine nor push worker start. Useful for tests and one-off operations. |

## Testing strategy

**Unit tests** (Minitest, no network):
- Signature verifier: valid, mutated body, mutated peer_id, skewed timestamp, unknown peer, blocked peer.
- Watermark calc: given a DB state, manifest returns expected watermarks; pull returns expected rows per `since`.
- Upsert semantics: first-write wins, first-tombstone wins, tombstone-before-content, late content for tombstoned global_id.
- Subscription filter: pull from un-subscribed caller returns empty; push queue filters peers without subscription.
- Chunk filter: rows with `memory_type='_chunk'` never appear in pull or push payloads.
- Re-embed enqueue: upserting a parent enqueues `(local_id, content, project)` exactly once.

**Integration tests** (two `HTTPServer` instances in-process, Faraday routed via in-memory Rack adapter):
- Two-peer pairing + initial sync.
- Push-on-write: write on A → record visible on B without manual sync.
- Pull-on-startup: A writes while B's engine paused; B starts, catches up.
- Tombstone propagates: delete on A → search excludes on B.
- Subscription enforcement: B subscribes to A's `global` only; A writes to `personal-finance`; B does not receive it.
- Replay idempotence: same record pushed twice = one row.
- Three-peer fan-out: A writes → B and C both end up consistent (via push) or by heartbeat from each other.
- Pagination: push 1500 records on A, B catches up via pull with `limit=500` over 3 round-trips.

**Smoke tests** (manual, end of phase):
- Two real `bin/server` processes on different ports + data dirs, paired, real writes propagate.

**Coverage:** must not regress below current baseline (currently 86.97% — should improve slightly given the volume of new tested code).

## Out-of-scope details left to the implementation plan

- Exact placement of the Sinatra `before` filter and how to share it with `/api/*` (which has its own loopback filter).
- Whether `Sync::Engine` and `Sync::PushQueue` are class-level singletons on `HTTPServer` (matches existing `db_manager` pattern — almost certainly yes).
- How tests start/stop background threads cleanly (`Recollect::TestCase` teardown hook).
- Faraday adapter selection wiring (likely a `Sync::Client.build` factory consulting `ENV` or `Recollect.config`).

These get nailed down in the Phase 2 implementation plan, which is the next document.
