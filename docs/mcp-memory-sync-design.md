# MCP Memory Sync Feature - Design Document

## Overview

This document describes the peer-to-peer synchronization feature for the MCP memory server. The server stores memory records in SQLite databases (one global, one per project). This sync feature enables multiple instances to share the same data across devices (e.g., PC and laptop).

### Key Characteristics

- Records are append-only (never modified after creation)
- Deletions are rare
- All peers are equal (no primary/replica distinction)
- Peers communicate directly via HTTP (no intermediate service)
- Network connectivity via Tailscale (stable hostnames, handles NAT traversal)

## Data Model

### Core Principle: Append-Only with Tombstones

Records are never modified or hard-deleted. Deletions are implemented as tombstones (soft delete). This makes sync deterministic - any peer can receive any record and simply insert it if missing.

### Schema Additions

```sql
-- Identity (generated once on first run)
CREATE TABLE local_identity (
    peer_id TEXT PRIMARY KEY,        -- derived from public key (e.g., base58 of first 16 bytes)
    display_name TEXT,               -- user-friendly name, e.g., "Jens's Laptop"
    public_key BLOB NOT NULL,
    private_key BLOB NOT NULL,       -- consider encrypting at rest
    created_at TEXT NOT NULL
);

-- Known peers in the sync network
CREATE TABLE known_peers (
    peer_id TEXT PRIMARY KEY,
    display_name TEXT,
    public_key BLOB NOT NULL,
    endpoint TEXT,                   -- Tailscale endpoint, e.g., "https://laptop.tailnet.ts.net:8080"
    status TEXT DEFAULT 'pending',   -- 'pending', 'trusted', 'blocked'
    invited_by TEXT,                 -- peer_id that introduced them (NULL if direct pairing)
    trusted_at TEXT,
    last_seen_at TEXT
);

-- Track sync progress per originating peer
CREATE TABLE peer_watermarks (
    peer_id TEXT PRIMARY KEY,        -- the peer that CREATED the records
    latest_created_at TEXT           -- latest record timestamp we have from this peer
);

-- Short-lived pairing codes
CREATE TABLE pairing_codes (
    code TEXT PRIMARY KEY,           -- e.g., "AXBF-2K9M"
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_by_peer TEXT                -- filled when consumed
);
```

### Modifications to Existing Tables

Add these columns to memory record tables:

```sql
ALTER TABLE memories ADD COLUMN origin_peer TEXT NOT NULL;
ALTER TABLE memories ADD COLUMN deleted_at TEXT;          -- NULL = active, timestamp = tombstone
ALTER TABLE memories ADD COLUMN deleted_by_peer TEXT;     -- who performed the deletion
```

Ensure the primary key is a UUID (preferably UUIDv7 for time-ordering).

## Peer Identity and Trust

### Identity Generation

On first startup, generate an Ed25519 keypair:

```
peer_id = base58(sha256(public_key)[:16])  -- short, URL-safe identifier
```

Store in `local_identity` table. This identity is permanent for the instance.

### Trust Model

- All peers are equal (full read/write/delete capability)
- Trust is established explicitly through pairing
- No automatic trust - every peer must be manually approved
- Blocked peers are rejected at the API layer

## Pairing Protocol

### Initiating Pairing (Device A)

```
POST /pairing/create
Authorization: (local request only, or require local auth)

Response:
{
  "code": "AXBF-2K9M",
  "expires_at": "2025-01-15T10:35:00Z",
  "endpoint": "https://pc.tailnet.ts.net:8080"
}
```

Display the code to the user. Code should be:
- 8 characters, alphanumeric, uppercase, hyphenated for readability
- Valid for 5 minutes
- Single use

### Joining (Device B)

```
POST /pairing/join
{
  "code": "AXBF-2K9M",
  "peer_id": "laptop-xyz",
  "display_name": "Jens's Laptop",
  "public_key": "<base64-encoded public key>",
  "endpoint": "https://laptop.tailnet.ts.net:8080"
}
```

### Approval Flow

Device A receives the join request and prompts the user:

```
Incoming pairing request:
  Peer ID: laptop-xyz
  Name: Jens's Laptop
  Endpoint: https://laptop.tailnet.ts.net:8080

Accept this peer? [y/N]:
```

On approval, Device A responds:

```json
{
  "accepted": true,
  "peer_id": "pc-abc123",
  "display_name": "Jens's PC",
  "public_key": "<base64-encoded public key>",
  "endpoint": "https://pc.tailnet.ts.net:8080",
  "known_peers": [
    // Optionally share other trusted peers for introduction
  ]
}
```

Both devices store each other as trusted and initiate first sync.

### Sequence Diagram

```
Device A (initiator)                      Device B (joiner)
       │                                        │
       │  User: "pair"                          │
       │  Generate code: AXBF-2K9M              │
       │  Display code                          │
       │                                        │
       │                              User: "join AXBF-2K9M --endpoint <A>"
       │                                        │
       │◄─────── POST /pairing/join ────────────┤
       │         {peer_id, public_key, ...}     │
       │                                        │
       │  Prompt user: "Accept? [y/N]"          │
       │  User: "y"                             │
       │                                        │
       ├─────── 200 OK ────────────────────────►│
       │        {peer_id, public_key, ...}      │
       │                                        │
       │  Both sides store peer as trusted      │
       │                                        │
       │◄─────── Initial sync begins ──────────►│
```

## Sync Protocol

### Endpoint 1: Get Manifest

Returns this peer's identity and watermarks (what it knows).

```
GET /sync/manifest
X-Peer-ID: <requesting peer's ID>
X-Timestamp: <ISO timestamp>
X-Signature: <signature of peer_id + timestamp>

Response:
{
  "peer_id": "pc-abc123",
  "watermarks": {
    "pc-abc123": "2025-01-15T10:30:00Z",
    "laptop-xyz": "2025-01-15T09:00:00Z",
    "server-789": "2025-01-14T22:00:00Z"
  }
}
```

Watermarks track the latest `created_at` timestamp seen from each originating peer.

### Endpoint 2: Get Records

Fetch records newer than the caller's watermarks.

```
GET /sync/records?since[pc-abc123]=2025-01-15T08:00:00Z&since[laptop-xyz]=...
X-Peer-ID: <requesting peer's ID>
X-Timestamp: <ISO timestamp>
X-Signature: <signature>

Response:
{
  "records": [
    {
      "id": "uuid-1",
      "origin_peer": "pc-abc123",
      "created_at": "2025-01-15T10:00:00Z",
      "content": "...",
      "deleted_at": null,
      "deleted_by_peer": null
      // ... other fields
    },
    {
      "id": "uuid-2",
      "origin_peer": "laptop-xyz",
      "created_at": "2025-01-15T09:30:00Z",
      "deleted_at": "2025-01-15T10:15:00Z",
      "deleted_by_peer": "pc-abc123"
    }
  ]
}
```

### Endpoint 3: Push Records

Push new/updated records to a peer.

```
POST /sync/records
X-Peer-ID: <sending peer's ID>
X-Timestamp: <ISO timestamp>
X-Signature: <signature of peer_id + timestamp + body_hash>

{
  "records": [...]
}

Response:
{
  "accepted": 15,
  "rejected": 0
}
```

### Record Insertion Logic

When receiving a record:

```sql
INSERT INTO memories (id, origin_peer, created_at, content, deleted_at, deleted_by_peer)
VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(id) DO UPDATE SET
    -- Only update tombstone fields, and only if we're applying a deletion
    deleted_at = COALESCE(memories.deleted_at, excluded.deleted_at),
    deleted_by_peer = CASE 
        WHEN memories.deleted_at IS NULL AND excluded.deleted_at IS NOT NULL 
        THEN excluded.deleted_by_peer 
        ELSE memories.deleted_by_peer 
    END;
```

Once deleted, a record stays deleted (tombstone is permanent).

### Watermark Update

After applying received records:

```sql
INSERT INTO peer_watermarks (peer_id, latest_created_at)
VALUES (?, ?)
ON CONFLICT(peer_id) DO UPDATE SET
    latest_created_at = MAX(peer_watermarks.latest_created_at, excluded.latest_created_at);
```

## Sync Triggers

### Push on Write

When a record is created or deleted locally, immediately push to all trusted peers:

```
for each peer in trusted_peers:
    try:
        POST peer.endpoint/sync/records with [new_record]
    catch connection_error:
        # Peer offline, they'll get it on their next pull
        log("Peer {peer.peer_id} unreachable, skipping push")
```

This ensures near-real-time propagation when peers are online.

### Pull on Startup

On server startup, sync with all trusted peers:

```
for each peer in trusted_peers:
    try:
        manifest = GET peer.endpoint/sync/manifest
        
        # Determine what we're missing
        records_needed = GET peer.endpoint/sync/records?since[...]=...
        apply(records_needed)
        
        # Push what they're missing
        records_to_send = find_records_newer_than(manifest.watermarks)
        POST peer.endpoint/sync/records with records_to_send
        
        update peer.last_seen_at
    catch connection_error:
        log("Peer {peer.peer_id} unreachable")
```

### No Periodic Sync Needed

With push-on-write and pull-on-startup:
- Online peers stay in sync via push
- Offline periods are reconciled on next startup

Periodic sync would be redundant.

## Request Authentication

All sync endpoints require authentication via request signing.

### Signature Scheme

```
signature_input = peer_id + timestamp + request_body_hash
signature = Ed25519.sign(signature_input, private_key)
```

### Headers

```
X-Peer-ID: laptop-xyz
X-Timestamp: 2025-01-15T10:30:00Z
X-Signature: <base64-encoded signature>
```

### Verification

1. Check `X-Peer-ID` is in `known_peers` with status = 'trusted'
2. Check `X-Timestamp` is within acceptable skew (e.g., ±5 minutes)
3. Verify signature against stored public key
4. Reject if any check fails

## Handling Edge Cases

### Peer Changes Endpoint

Tailscale provides stable DNS names, so this should be rare. If a peer's endpoint changes:

1. Peer includes current endpoint in signed requests
2. Receiver updates stored endpoint if signature valid
3. Alternatively, use Tailscale MagicDNS names which don't change

### Peer Goes Offline

- Push attempts fail silently (logged)
- Offline peer catches up via pull-on-startup when back online
- Watermark system ensures no records are missed

### Conflicting Deletes

Not possible - deletions only add a tombstone. If two peers delete the same record:
- Both set `deleted_at` (may differ by seconds)
- First tombstone wins on merge (COALESCE keeps first non-null)
- End state is the same: record is deleted

### Large Initial Sync

For first sync with many records, consider:
- Pagination in `/sync/records` (limit + offset or cursor)
- Progress indication to user
- Chunked transfer

### Lost/Compromised Device

1. Mark peer as blocked on all other devices
2. Optionally propagate blocks via sync (add `blocked_peers` table)
3. Blocked peer's future requests are rejected
4. Historical data from that peer remains (was valid at time of sync)

## API Summary

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/pairing/create` | POST | Generate pairing code (local only) |
| `/pairing/join` | POST | Request to join using code |
| `/sync/manifest` | GET | Get peer's identity and watermarks |
| `/sync/records` | GET | Fetch records newer than given watermarks |
| `/sync/records` | POST | Push records to peer |
| `/peers` | GET | List known peers (local only) |
| `/peers/:id` | DELETE | Block/remove peer (local only) |

## CLI Commands

```bash
# Show local identity
mcp-memory identity

# Initiate pairing
mcp-memory pair
# → displays code and waits for join request

# Join existing network
mcp-memory join <CODE> --endpoint <PEER_ENDPOINT>

# List peers
mcp-memory peers

# Remove/block peer
mcp-memory peers remove <PEER_ID>

# Manual sync (usually not needed)
mcp-memory sync [--peer <PEER_ID>]
```

## Implementation Notes

### Dependencies

- HTTP client for peer communication
- Ed25519 library for keypair generation and signing
- UUID library (preferably UUIDv7)
- JSON serialization

### Database Migrations

1. Add `local_identity` table
2. Add `known_peers` table
3. Add `peer_watermarks` table
4. Add `pairing_codes` table
5. Add `origin_peer`, `deleted_at`, `deleted_by_peer` columns to memory tables
6. Backfill existing records with local peer as `origin_peer`

### Security Considerations

- Private key should be encrypted at rest (or rely on OS keychain)
- Pairing codes are single-use and short-lived
- All sync requests are signed and verified
- Consider rate limiting on pairing endpoints
- Tailscale already provides encryption in transit
