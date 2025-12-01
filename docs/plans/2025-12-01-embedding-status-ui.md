# Embedding Status in Web UI

## Overview

When vector search is enabled, show embedding status in the web UI so users can see coverage and identify memories pending embedding.

## Requirements

1. **Status bar at top** - Shows overall embedding coverage (e.g., "42/100 (42%)")
2. **Per-memory indicator** - Icon (⏳) on cards missing embeddings, with tooltip
3. **Conditional display** - Only show when vectors are enabled

## Implementation

### API Changes

The `/api/vectors/status` endpoint already exists and returns:
- When enabled: `{enabled: true, total_memories: N, total_embeddings: M, coverage: X}`
- When disabled: `{enabled: false, reason: "..."}`

Need to extend `/api/memories` to include `has_embedding` boolean per memory when vectors are enabled.

### Frontend Changes

**On page load:**
1. Fetch `/api/vectors/status`
2. If `enabled: true`, show status bar and track that we need embedding indicators
3. If `enabled: false`, do nothing

**Status bar HTML:**
```html
<div id="embeddingStatus" class="embedding-status">
  🔢 Embeddings: <span id="embeddingCount">0/0</span> (<span id="embeddingCoverage">0</span>%)
</div>
```

**Per-memory indicator:**
Add to memory card header when `has_embedding: false`:
```html
<span class="embedding-pending" title="Pending embedding">⏳</span>
```

### CSS

Subtle styling for status bar - muted background, not attention-grabbing.
