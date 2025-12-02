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
