---
name: using-long-term-memory
description: Use when you have access to memory/journal tools (recollect, episodic-memory, etc.) - ensures proactive storage of decisions and learnings, and searching memory BEFORE asking questions when encountering problems
---

# Using Long-Term Memory

## Overview

You have memory tools but won't use them proactively without discipline. **Search before asking. Store before moving on.**

## Core Rules

### Retrieval: Search FIRST

**When you encounter a problem, error, or unfamiliar situation:**

1. Search memory BEFORE asking the user questions
2. Search memory BEFORE investigating the codebase
3. Only proceed to other approaches if memory search yields nothing relevant

**No exceptions for urgency.** Production down? Search takes 2 seconds. Emergency? Search first anyway. The memory might contain the exact fix. Skipping search to "save time" often costs more time.

**Trigger phrases in your own thinking:**
- "I've never seen this before" → Search memory, you might have
- "Let me ask which..." → Search memory first
- "I need more context" → Search memory first
- "This is urgent" → Search memory, it's fast

### Storage: Store BEFORE Moving On

**When any of these happen, store immediately:**

| Event | Action |
|-------|--------|
| Decision made | Store with tags: decision, [topic] |
| Lesson learned | Store with tags: learning, [topic] |
| Bug solved | Store with tags: bug, [symptom] |
| User preference discovered | Store with tags: preference, [topic] |
| Architecture choice | Store with tags: architecture, [component] |

**Do not** say "I should store this" and then move on. Actually call the tool.

**What counts as a decision?** If you discussed trade-offs, considered alternatives, or the choice affects future work → store it. Routine refactors (renaming a variable, extracting a method) with no discussion → skip.

### Granularity: Project vs Global

| Store Globally | Store in Project |
|----------------|------------------|
| User preferences | Architecture decisions |
| Cross-project patterns | Tech stack choices |
| Working style | Project-specific conventions |
| Tool preferences | Known issues in this codebase |

**Default to project-specific.** Only use global for things that clearly apply everywhere.

## Choosing the Interface: MCP Tools vs CLI

Recollect exposes both MCP tools and a CLI; they talk to the same server. Pick by task shape:

**MCP tools** (`store_memory`, `search_memory`, `get_context`, ...) are the default for the ambient flow above — storing a decision mid-conversation, a quick recall search. Lowest friction, no shell round-trip.

**CLI** (`recollect`, server URL via `RECOLLECT_URL`, default `http://localhost:7326`) wins when results feed further processing — bulk reads, filtering, anything you would pipe into `jq`. Two rules: table output truncates content, so pass `--json` whenever you need the actual text; a nonzero exit code means the command failed.

| Task | Command |
|------|---------|
| Search with full content | `recollect search "auth bug" --json` |
| Read one memory in full | `recollect show 42` (`-p project`, `--json`) |
| Memories matching ALL tags | `recollect find-by-tag decision,auth --json` |
| Recent memories in a project | `recollect list -p myproj --json` |
| Tag / project inventory | `recollect tags --json`, `recollect projects --json` |
| Store (scripted) | `recollect store "content" -p proj -t decision -T tag1,tag2` |
| Delete | `recollect delete 42 [-p project]` |

Example — reduce results before they hit your context:

```bash
recollect search "deploy" --json | jq -r '.results[] | "\(.id): \(.content)"' | head -20
```

## Red Flags - You're About to Fail

- Asking user a question without searching memory first
- Saying "noted" or "I'll remember that" without calling store tool
- Debugging an error without checking if it was solved before
- Moving to next task after a decision without storing it
- Skipping search because "it's urgent" or "production is down"
- Thinking "I already know how to fix this" without searching

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "It's urgent, no time to search" | Search takes 2 seconds. Emergency is when memory helps most. |
| "I already know the fix" | Memory might have project-specific context you're missing. |
| "This is too trivial to store" | Did you discuss trade-offs? If yes, store it. |

## Quick Reference

```
Error/problem encountered → search_memory(query="[error or symptom]")
Decision just made → store_memory(content="...", tags=["decision", ...])
Learned something → store_memory(content="...", tags=["learning", ...])
User preference → store_memory(content="...", tags=["preference", ...], project=nil)
```
