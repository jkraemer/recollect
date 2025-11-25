# Tags Migration Implementation Plan

## Overview

Migrate from rigid 5-type system (`note`, `decision`, `pattern`, `bug`, `learning`) to flexible 2-type + tags system (`note`, `todo` + unlimited tags).

**Scope**: Full implementation without migration script (fresh database).
**Breaking Change**: Old types rejected after implementation.

## Critical Files to Modify

| File | Changes |
|------|---------|
| `lib/recollect/tools/store_memory.rb` | Update enum to `['note', 'todo']`, enhance description |
| `lib/recollect/tools/search_memory.rb` | Update enum, add tags filter parameter |
| `lib/recollect/database.rb` | Add `search_by_tags()` and `get_tag_stats()` methods |
| `lib/recollect/database_manager.rb` | Add tag stats aggregation |
| `lib/recollect/http_server.rb` | Add `/api/tags` and `/api/memories/by-tags` endpoints |
| `bin/recollect` | Add `tags` and `find-by-tag` commands |

## Implementation Steps

### Step 1: Database Layer - Add Tag Methods

**File**: `lib/recollect/database.rb`

Add two new methods:

```ruby
def search_by_tags(tag_filters, memory_type: nil, limit: 10)
  # Filter memories that contain ALL specified tags (AND logic)
  # Tags stored as JSON array - use LIKE for substring matching
end

def get_tag_stats(memory_type: nil)
  # Parse all tags from memories, count frequency
  # Return hash of { tag => count } sorted by frequency desc
end
```

### Step 2: Database Manager - Tag Stats Aggregation

**File**: `lib/recollect/database_manager.rb`

Add method to aggregate tag stats across all databases:

```ruby
def tag_stats(project: nil, memory_type: nil)
  # If project specified, get stats from that db only
  # Otherwise aggregate across global + all project dbs
end
```

### Step 3: Update MCP Tools

**File**: `lib/recollect/tools/store_memory.rb`
- Change enum from `%w[note decision pattern bug learning]` to `%w[note todo]`
- Update description to reflect new tagging philosophy
- Add SUGGESTED_TAGS constant for documentation

**File**: `lib/recollect/tools/search_memory.rb`
- Change enum from `%w[note decision pattern bug learning]` to `%w[note todo]`
- Add `tags` parameter (array) for filtering by specific tags
- Update description with tag search guidance

### Step 4: HTTP API Endpoints

**File**: `lib/recollect/http_server.rb`

Add two new endpoints:

```ruby
# GET /api/tags?project=X&memory_type=note
# Returns: { tags: { "decision": 5, "threading": 3 }, total: 8, unique: 2 }

# GET /api/memories/by-tags?tags=decision,threading&project=X&memory_type=note&limit=10
# Returns: { results: [...], count: N, tags: ["decision", "threading"] }
```

### Step 5: CLI Commands

**File**: `bin/recollect`

Add two new commands:

```ruby
# recollect tags [-p project] [-t type] [-n top_count]
# Shows tag frequency statistics with visual bar chart

# recollect find-by-tag TAGS [-p project] [-t type] [-l limit]
# Finds memories matching ALL specified tags (comma-separated)
```

### Step 6: Update Tests

Create/update test files:
- `test/recollect/database_test.rb` - Add tests for `search_by_tags` and `get_tag_stats`
- `test/recollect/tools/store_memory_test.rb` - Update type enum tests
- `test/recollect/tools/search_memory_test.rb` - Add tag filter tests
- `test/recollect/http_server_test.rb` - Add endpoint tests

## Test Cases

### Database Tests
1. `search_by_tags` returns memories matching ALL tags
2. `search_by_tags` with type filter works
3. `search_by_tags` returns empty array when no matches
4. `get_tag_stats` counts tag frequency correctly
5. `get_tag_stats` with type filter works

### Tool Tests
1. `store_memory` rejects old types (decision, pattern, bug, learning)
2. `store_memory` accepts 'note' and 'todo'
3. `search_memory` with tags filter works
4. `search_memory` combines query + tags filter

### API Tests
1. GET `/api/tags` returns correct stats
2. GET `/api/memories/by-tags` filters correctly
3. POST `/api/memories` rejects old types

## Order of Implementation

1. **Database layer first** (foundation)
   - Add `search_by_tags()` to Database
   - Add `get_tag_stats()` to Database
   - Add tests

2. **Database manager** (aggregation)
   - Add `tag_stats()` method
   - Add tests

3. **MCP tools** (API contract)
   - Update store_memory enum + description
   - Update search_memory enum + add tags param
   - Update tests

4. **HTTP endpoints** (REST API)
   - Add `/api/tags` endpoint
   - Add `/api/memories/by-tags` endpoint
   - Add tests

5. **CLI commands** (user interface)
   - Add `tags` command
   - Add `find-by-tag` command
   - Manual testing

## Notes

- **Tags are case-insensitive**: Normalize to lowercase on store and search
- Tag matching uses substring search against JSON array (current FTS5 approach)
- No schema changes needed - existing `tags TEXT` column works
- Breaking change: old types rejected, not silently converted

## Case Normalization

Tags will be normalized to lowercase at these points:
1. `Database#store` - normalize tags array before JSON encoding
2. `Database#search_by_tags` - normalize filter tags before matching
3. `Database#get_tag_stats` - normalize when counting (or rely on stored normalization)
