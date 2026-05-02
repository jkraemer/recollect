# Recollect Sync Design

Status: approved, ready for implementation planning
Date: 2026-05-02

This design supersedes the earlier draft in `docs/mcp-memory-sync-design.md`. It integrates decisions made during brainstorming on 2026-04-29 / 2026-05-02.

## Goals

Enable a user's recollect instance to share memories across multiple machines (e.g., desktop and laptop) with no manual sync action after initial pairing. Each machine runs a full peer; there is no central server. Sync is ambient: changes propagate automatically when peers are reachable, and missed changes catch up on next contact.

## Non-Goals

- Public, untrusted-network deployment. We assume Tailscale (or equivalent VPN) provides transport encryption and stable hostnames.
- Web UI for peer management (CLI is enough for v1).
- HTTPS / TLS termination (Tailscale already encrypts).
- Encrypted-at-rest private keys (filesystem permissions).
- Vacuum / permanent deletion of tombstones.
- Re-embedding when the embedding model changes.

## Architecture overview

Three databases, three roles:

```
~/.recollect/
├── sync.db                  # identity, peers, watermarks, pairing. Never travels.
├── global.db                # cross-project memories. Syncs.
└── projects/
    └── <name>.db            # per-project memories. Each syncs independently.
```

`sync.db` is a structural privacy boundary: data that travels (memory records) is in different files from data about how it travels (peer keys, trust state). Filesystem permissions on `sync.db` are 0600.

A new `Sync` module sits alongside `MemoriesService`:

- `Sync::Identity` — local Ed25519 keypair, peer_id derivation
- `Sync::Crypto` — request signing and verification helpers
- `Sync::Peers` — trusted peer registry
- `Sync::Engine` — orchestrates pull-on-startup, heartbeat reconciliation, push-on-write fan-out
- `Sync::PushQueue` — bounded in-process queue + worker thread, drained by Engine

HTTP endpoints `/pairing/*` and `/sync/*` are added to the existing `HTTPServer`. Same Puma process; no new daemon, no new transport.

## Record identity

The current schema uses `id INTEGER PRIMARY KEY AUTOINCREMENT`, which is local-only. We add a globally unique identifier without disturbing the local primary key:

```sql
ALTER TABLE memories ADD COLUMN global_id TEXT;
ALTER TABLE memories ADD COLUMN origin_peer TEXT;
ALTER TABLE memories ADD COLUMN deleted_at TEXT;
ALTER TABLE memories ADD COLUMN deleted_by_peer TEXT;

UPDATE memories
SET global_id   = <generated UUIDv7>,
    origin_peer = <local peer_id>
WHERE global_id IS NULL;

CREATE UNIQUE INDEX idx_memories_global_id      ON memories(global_id);
CREATE INDEX        idx_memories_origin_created ON memories(origin_peer, created_at);
CREATE INDEX        idx_memories_deleted        ON memories(deleted_at) WHERE deleted_at IS NOT NULL;
```

`global_id` is UUIDv7 (time-ordered). Local code keeps using `INTEGER id`; only the sync layer touches `global_id`. This avoids rewriting FTS rowid coupling, vec_memories, and every join in the codebase.

## Tombstones

Deletes set `deleted_at` and `deleted_by_peer`; rows are never hard-deleted. Two triggers handle index cleanup on the live → deleted transition:

```sql
-- Always installed
CREATE TRIGGER memories_tombstone_fts AFTER UPDATE ON memories
WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL
BEGIN
  DELETE FROM memories_fts WHERE rowid = NEW.id;
END;

-- Installed only when sqlite-vec extension loads
CREATE TRIGGER memories_tombstone_vec AFTER UPDATE ON memories
WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL
BEGIN
  DELETE FROM vec_memories WHERE rowid = NEW.id;
END;
```

(SQLite cannot defer-resolve a table reference inside a trigger; an `EXISTS` guard against `sqlite_master` does not help. Two triggers, one always-on for FTS, one installed alongside the vector extension, is the cleanest path.)

All search/list/get queries get an additional `WHERE deleted_at IS NULL` clause. Tombstoned rows remain on disk for sync convergence but are invisible to queries.

## sync.db schema

```sql
CREATE TABLE local_identity (
  peer_id      TEXT PRIMARY KEY,
  display_name TEXT,
  public_key   BLOB NOT NULL,
  private_key  BLOB NOT NULL,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE known_peers (
  peer_id         TEXT PRIMARY KEY,
  display_name    TEXT,
  public_key      BLOB NOT NULL,
  endpoint        TEXT NOT NULL,
  status          TEXT NOT NULL DEFAULT 'trusted',  -- 'trusted' | 'blocked'
  trusted_at      TEXT NOT NULL,
  last_seen_at    TEXT,
  last_sync_at    TEXT,
  last_sync_error TEXT
);

CREATE TABLE peer_watermarks (
  peer_id           TEXT NOT NULL,   -- the peer that CREATED the records (origin)
  db_name           TEXT NOT NULL,   -- 'global' or project name
  latest_created_at TEXT NOT NULL,
  PRIMARY KEY (peer_id, db_name)
);

CREATE TABLE pairing_codes (
  code         TEXT PRIMARY KEY,
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  used_at      TEXT,
  used_by_peer TEXT
);

-- Sender-side outbound sharing policy: which of MY DBs go to which peer.
-- Default after pairing: one row per peer with db_name='global'.
CREATE TABLE peer_db_subscriptions (
  peer_id TEXT NOT NULL,
  db_name TEXT NOT NULL,
  PRIMARY KEY (peer_id, db_name)
);
```

`peer_db_subscriptions` is the privacy boundary. The sender filters by it on push and on pull responses. The receiver does not filter — if you sent it to me, I keep it. This means subscription changes are unilateral and require no peer coordination, but the privacy semantics still hold (data only leaves the sender if the sender's policy allows).

## Identity and peer_id

On first run, generate an Ed25519 keypair and derive:

```
peer_id = base58(sha256(public_key)[:16])
```

`display_name` defaults to the machine's hostname; user can override via `recollect identity --rename`.

Private key is stored unencrypted in `sync.db`. `sync.db` is chmod 0600. For a single-user local daemon this matches the threat model — encryption-at-rest only matters if disk access is untrusted, in which case the user should use full-disk encryption.

## Endpoint discovery

When a peer needs to publish its own URL (e.g., during pairing), discovery proceeds in this order:

1. `RECOLLECT_PUBLIC_URL` env var if set. Always wins.
2. `tailscale status --json` parsed for the local node's MagicDNS name + the configured port.
3. First non-loopback, non-link-local IPv4 from `Socket.ip_address_list` + the configured port.
4. Hard error: "Could not auto-detect a reachable URL. Set RECOLLECT_PUBLIC_URL or pass --endpoint."

URLs are `http://`, not `https://`. Tailscale provides transport encryption; application-layer signatures provide authentication. For non-Tailscale internet exposure, a reverse proxy is the user's responsibility.

## Pairing protocol

Single-step trust: the pairing code is the security token. No second confirmation.

```
Device A (initiator)                          Device B (joiner)
─────────────────────                         ────────────────
$ recollect pair
→ POST /pairing/create  (local-only: 127.0.0.1/::1)
  ← { code, expires_at, endpoint }
Print code + join command + endpoint.
Exit. (Pairing happens server-side; user verifies via `recollect peers`.)

                                              $ recollect join AXBF-2K9M --endpoint http://...
                                              → POST <A>/pairing/join
                                                { code, peer_id, display_name,
                                                  public_key, endpoint }

  Validate code: exists, not expired, not used.
  Mark code used (used_at, used_by_peer).
  Insert B into known_peers (status='trusted',
    trusted_at=now).
  Insert default subscription: (B, 'global').
  ← 200 OK
    { peer_id, display_name, public_key, endpoint }

                                              Insert A into known_peers (trusted).
                                              Insert default subscription: (A, 'global').
                                              Trigger initial sync for db='global'.
```

Pairing codes:
- 8 characters, uppercase alphanumeric, hyphenated for readability (e.g., `AXBF-2K9M`).
- 5-minute TTL.
- Single use (`used_at` non-NULL = spent).

`/pairing/create` is local-only: rejected unless the request originates from `127.0.0.1` or `::1`. Prevents a remote attacker who reaches the port from minting codes.

## Sync protocol

All `/sync/*` requests are signed. Three endpoints. Sync runs **per (peer, db_name)** — the unit of work is one peer + one DB.

### Request signing

Headers:

```
X-Peer-ID:    <sender's peer_id>
X-Timestamp:  <ISO 8601 UTC>
X-Body-Hash:  <sha256(body) hex; empty-string sha256 for GET>
X-Signature:  <base64 Ed25519 signature of "peer_id\ntimestamp\nbody_hash">
```

Verify:
1. `X-Peer-ID` exists in `known_peers` with `status='trusted'`.
2. `X-Timestamp` within ±5 minutes of local clock.
3. `X-Signature` valid against the stored public key.
4. Update `known_peers.last_seen_at`.

Reject with 401 (invalid signature/timestamp/unknown peer) or 403 (blocked peer).

### `GET /sync/manifest?db=<db_name>`

Receiver's view of what it has for one DB:

```json
{
  "peer_id": "pc-abc123",
  "db_name": "global",
  "watermarks": {
    "pc-abc123":  "2026-05-01T10:30:00.000Z",
    "laptop-xyz": "2026-04-30T09:00:00.000Z",
    "server-789": "2026-04-29T22:00:00.000Z"
  }
}
```

Self-watermark is `MAX(created_at)` from rows where `origin_peer = local_peer_id` in that DB. Other watermarks come from `peer_watermarks` rows for that `db_name`.

### `POST /sync/pull?db=<db_name>`

Body:

```json
{
  "since": {
    "pc-abc123":  "2026-05-01T10:30:00.000Z",
    "laptop-xyz": "2026-04-30T09:00:00.000Z"
  },
  "limit": 500
}
```

Returns up to `limit` records from the requested DB where `(origin_peer, created_at)` exceeds the caller's watermark for that origin, ordered by `created_at` ascending. Includes tombstones. Embedding BLOB included as base64. Filtered by sender's `peer_db_subscriptions` (caller does not receive records from DBs the sender does not share with them).

If `count == limit`, caller pulls again with updated watermarks until response is short.

### `POST /sync/push?db=<db_name>`

Body:

```json
{ "records": [ /* same shape as pull response */ ] }
```

Receiver upserts each record:

```sql
INSERT INTO memories (global_id, origin_peer, created_at, content, ...,
                      deleted_at, deleted_by_peer)
VALUES (?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(global_id) DO UPDATE SET
  deleted_at      = COALESCE(memories.deleted_at, excluded.deleted_at),
  deleted_by_peer = CASE
    WHEN memories.deleted_at IS NULL AND excluded.deleted_at IS NOT NULL
    THEN excluded.deleted_by_peer
    ELSE memories.deleted_by_peer
  END;
```

First-write wins for content; first-tombstone wins for deletion. Late-arriving content for an already-tombstoned UUID does not un-tombstone.

After applying records, advance `peer_watermarks` for each origin observed:

```sql
INSERT INTO peer_watermarks (peer_id, db_name, latest_created_at)
VALUES (?, ?, ?)
ON CONFLICT(peer_id, db_name) DO UPDATE SET
  latest_created_at = MAX(peer_watermarks.latest_created_at, excluded.latest_created_at);
```

Returns `{ accepted: N, rejected: 0 }`.

### Full exchange between A and B for one DB

```
1. A → GET  /sync/manifest?db=global       (sees what B has)
2. A → POST /sync/push?db=global           (sends records B is missing)
3. A → POST /sync/pull?db=global           (asks for records A is missing)
4. A advances peer_watermarks based on records received
5. B advanced its watermarks during step 2
```

Push-on-write does only step 2 with a single record. Pull-on-startup and heartbeat reconciliation do the full 1–4 sequence per (peer, subscribed-db).

## Ambient sync triggers

**Push-on-write.** After `MemoriesService` commits a store or delete, enqueue the `(global_id, db_name)` into `Sync::PushQueue`. A worker thread drains the queue: for each entry, look up trusted peers whose `peer_db_subscriptions` includes `db_name`, fire `POST /sync/push` to each in parallel. Failures log + set `last_sync_error` and drop from the queue. The heartbeat catches anything missed.

**Pull-on-startup.** During `HTTPServer` boot, spawn a `Sync::Engine` thread (concurrent with serving requests — startup must not block on a slow peer). For each trusted peer, for each subscribed DB: do the manifest+pull+push exchange. Update `last_sync_at` / `last_sync_error`. Errors are logged.

**Heartbeat reconciliation.** `Sync::Engine` runs a loop. Every `RECOLLECT_SYNC_HEARTBEAT_SECONDS` (default 300):
- For each trusted peer, for each subscribed DB: GET manifest. If their watermarks indicate they have records we lack, pull. If our own-origin watermark exceeds theirs, push (covers push-on-write failures). Update `last_seen_at`.
- Skip peers we've successfully synced with in the last heartbeat-interval/2 (cheap dedup).

The heartbeat is the discovery + liveness mechanism. Tailscale provides reachability; we don't need broadcast or relay.

## Error handling and edge cases

**Network errors during push/pull.** Logged with peer_id + endpoint + error. Stored in `known_peers.last_sync_error`. Cleared on next success. Never raised to `MemoriesService` callers.

**Signature verification failures.** 401 with no body detail. Logged at warn. Repeated failures from a known-trusted peer are not auto-blocked — visible in logs and `recollect peers`.

**Clock skew.** ±5 min window on `X-Timestamp`. Mention in `recollect pair` output: "if syncing fails with 'timestamp out of range', check clocks." We do not implement NTP correction.

**Replay protection.** Body hash + timestamp in signature is sufficient. No nonce cache. Replayed pushes are idempotent (same UUID upsert); replayed pulls are reads.

**Large initial sync.** Pull is paginated (limit=500). Caller iterates until response is short. 50k records over 100 round-trips is acceptable. Per-peer record-pulled counts shown in `recollect peers` is nice-to-have, not required.

**Tombstone for unknown UUID.** Insert as a tombstone row anyway (deleted_at set, content NULL/empty). When the actual content arrives later, upsert keeps the tombstone fields. End state: deleted.

**Record from unknown origin_peer.** `origin_peer` is a watermark label, not an authorization claim. Auth happens at the request signature (we trust the sender; sender vouches for content). Accept the record and advance our `peer_watermarks` for that origin.

**Blocked peer.** `recollect peers remove <id>` sets `status='blocked'`, keeping the public key for historical signature verification and preventing silent re-pair. Blocked peers' signed requests rejected with 403.

**Daemon restart mid-sync.** Watermarks advance only after committed local writes. Crash mid-pull means next heartbeat re-pulls from last advanced watermark.

## CLI surface

```
recollect identity [--rename <NAME>]
  Show local peer_id, display_name, public_key fingerprint, endpoint.

recollect pair
  Generate a pairing code. Print code, join command, endpoint, expiry.
  Exit immediately. Pairing happens when the other side joins;
  user verifies with `recollect peers`.

recollect join <CODE> --endpoint <PEER_ENDPOINT>
  Send join request, set local trust, kick off initial sync.

recollect peers
  List trusted peers: peer_id, display_name, endpoint, last_seen_at,
  last_sync_at, last_sync_error, subscribed DBs.

recollect peers remove <PEER_ID>
  Mark peer as blocked.

recollect peers add-db <PEER_ID> <DB_NAME>
recollect peers remove-db <PEER_ID> <DB_NAME>
  Manage outbound subscriptions (which of MY DBs go to this peer).

recollect sync [--peer <PEER_ID>] [--db <DB_NAME>]
  Force an immediate sync cycle. Debug-only; not in main --help.
```

## Testing strategy

**Unit tests:**
- Schema migrations: idempotent, backfill correct.
- Signing / verification: valid payload, mutated payload, mutated timestamp out of window.
- Watermark logic: given records + watermark map, "what to send" is correct.
- Tombstone trigger: FTS row removed (and vec row, when vectors loaded).
- Upsert logic: first-write wins for content, first-tombstone wins for deletion, late content does not un-tombstone.
- Pairing code lifecycle: generate, consume, expire, double-use rejected.

**Integration tests** (single Ruby process, two `Database` + `SyncStore` instances, talking via in-process HTTP using `Rack::Test` or two `HTTPServer` instances on ephemeral ports):
- Two-instance pairing handshake.
- Push-on-write propagates A → B.
- Pull catches up B after A wrote while B's engine was paused.
- Tombstone propagates; search excludes on both sides.
- Subscription filter: A does not push records from a non-shared DB to B.
- Replay/idempotence: same record pushed twice is a no-op.
- Out-of-order tombstone (tombstone before content) converges to deleted.
- Three-peer fan-out: A writes, both B and C end up consistent.

**End-to-end / smoke (manual, before each release):**
- Two real `bin/server` instances on different ports + `RECOLLECT_DATA_DIR`s, paired via CLI. Observe via `recollect peers` and `recollect search`.

**Test isolation.** Existing `RACK_ENV=test` setup uses `test/tmp/test_data`. Sync tests use `test/tmp/sync_test/{a,b}/` per peer. Cleaned between tests.

**Coverage target.** No regression from current 86.97%.

## Phasing

**Phase 1 — Foundation** (no network behavior, pure local refactor + pairing handshake)

1. Schema migration on memory DBs: `global_id`, `origin_peer`, `deleted_at`, `deleted_by_peer`. Backfill.
2. New `~/.recollect/sync.db` with all sync metadata tables. File mode 0600.
3. Identity generation on first run.
4. Endpoint auto-detection + `RECOLLECT_PUBLIC_URL` override.
5. `delete_memory` switches from hard-delete to tombstone.
6. Two tombstone triggers (FTS always, vec with extension).
7. `WHERE deleted_at IS NULL` on all read queries.
8. `Sync::Crypto` (signing + verification helpers).
9. `/pairing/create` (local-only) and `/pairing/join` endpoints.
10. CLI: `identity`, `pair`, `join`, `peers` (list / remove / add-db / remove-db).

End of Phase 1: pair two instances, see them in `recollect peers`. No records flow.

**Phase 2 — Ambient sync** (network protocol + automatic triggers)

1. `/sync/manifest`, `/sync/pull`, `/sync/push` endpoints with signature verification.
2. `Sync::Engine`: pull-on-startup thread, heartbeat reconciliation loop.
3. `Sync::PushQueue` + worker thread; push-on-write hook in `MemoriesService`.
4. Outbound subscription filter honored on push and on pull responses.
5. Embedding BLOB included in record payloads (base64).
6. Tombstone upsert semantics (COALESCE for first-tombstone-wins).
7. `last_sync_at` / `last_sync_error` tracking.
8. CLI: `sync` debug command added.

End of Phase 2: paired instances stay converged ambiently. Subscription policy enforced.

## Out of scope for v1

- Web UI for peer management
- Automated multi-process tests in CI
- Vacuum / permanent deletion of old tombstones
- Re-embed pass for embedding model changes
- HTTPS / cert management
- Encrypted-at-rest private key
- Conflict observability dashboards
- Periodic sync beyond heartbeat reconciliation

## Environment variables (additions)

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_PUBLIC_URL` | (auto-detect) | Override the externally-reachable URL published during pairing. |
| `RECOLLECT_SYNC_HEARTBEAT_SECONDS` | `300` | Heartbeat reconciliation interval. |
| `RECOLLECT_SYNC_PULL_LIMIT` | `500` | Records per pull page. |
| `RECOLLECT_SYNC_TIMESTAMP_SKEW_SECONDS` | `300` | Acceptable clock skew for signed-request timestamps. |

## Dependencies (additions)

- Ed25519 library (e.g., `ed25519` gem)
- UUIDv7 generator (small, can implement inline against `securerandom`)
- HTTP client for peer communication (`Net::HTTP` is fine; no need for a third-party gem)
- Base58 encoding for peer_id (small helper, can implement inline)
