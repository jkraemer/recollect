# Plan: Add Session Memory Type and /session-log Command

## Overview

Add `session` as a third memory type in recollect, and create a `/session-log` slash command that stores structured session summaries to long-term memory.

## Part 1: Add `session` Memory Type

### 1.1 Update Tool Schemas

**File: `lib/recollect/tools/store_memory.rb`** (line 56-59)
- Change enum from `%w[note todo]` to `%w[note todo session]`

**File: `lib/recollect/tools/search_memory.rb`** (line 62-65)
- Change enum from `%w[note todo]` to `%w[note todo session]`

### 1.2 Update Tests

**File: `test/recollect/tools/store_memory_test.rb`**
- Add test for storing with `memory_type: "session"`
- Update any tests that enumerate valid types

**File: `test/recollect/tools/search_memory_test.rb`**
- Add test for searching/filtering by `memory_type: "session"`

### 1.3 Update Web UI Styling

**File: `public/style.css`** (around line 81-84)
- Add `.type-session` styling (suggest: distinct color like teal/cyan)
- Also clean up stale old type styles (decision, pattern, bug, learning) that are no longer used

## Part 2: Create `/session-log` Slash Command

### 2.1 Create Command File

**File: `docs/claude/commands/session-log.md`**

Command content (adapted from existing session-logging skill):

```markdown
# Session Log

Create a session summary and store it in long-term memory for future retrieval.

## Instructions

1. Review the current conversation and identify:
   - What was worked on
   - Key decisions made
   - Problems solved
   - Current state of work
   - Logical next steps

2. Create a structured summary following this format:

## Session Summary Template

Session: [Descriptive Title]
Date: [Current UTC timestamp]

### Overview
[2-3 sentences summarizing what was accomplished]

### Key Decisions
- [Decision and reasoning]

### Problems Solved
- [Problem]: [Solution]

### Current State
[What's working, what's partial, what's broken]

### Next Steps
1. [Immediate next action]
2. [Following action]

### Context for Continuation
[Anything a future session needs to know to continue seamlessly]

3. Store the summary using the memory tool:
   - memory_type: "session"
   - tags: ["session", relevant topic tags]
   - project: current project name (or omit for cross-project sessions)

4. Confirm storage to the user with the memory ID.

$ARGUMENTS
```

### 2.2 Symlink for Personal Use

Create symlink from `~/.claude/commands/session-log.md` pointing to `docs/claude/commands/session-log.md` (or copy if preferred).

## Part 3: Delete Old Session-Logging Skill

**Directory: `~/.claude/skills/session-logging/`**

Delete entirely - the command replaces it.

## Files to Modify

| File | Change |
|------|--------|
| `lib/recollect/tools/store_memory.rb` | Add "session" to enum |
| `lib/recollect/tools/search_memory.rb` | Add "session" to enum |
| `test/recollect/tools/store_memory_test.rb` | Add session type tests |
| `test/recollect/tools/search_memory_test.rb` | Add session type tests |
| `public/style.css` | Add .type-session, remove stale styles |
| `docs/claude/commands/session-log.md` | Create new command |
| `~/.claude/skills/session-logging/` | Delete directory |

## Verification

1. Run `bundle exec rake test` - all tests pass
2. Run `bundle exec rubocop` - no offenses
3. Test manually:
   - Store a memory with `memory_type: "session"`
   - Search for it with type filter
   - Check UI displays session badge correctly
4. Test `/session-log` command in a new Claude Code session
