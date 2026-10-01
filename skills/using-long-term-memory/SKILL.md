---
name: using-long-term-memory
description: Use when the recollect CLI is available or a Recollect memory block appeared at session start - search memory BEFORE asking questions or investigating problems, and store decisions, learnings and solved bugs before moving on
---

# Using Long-Term Memory

## Overview

Recollect is a searchable, cross-project log of decisions, learnings, solved bugs and session summaries, used through the `recollect` command. You won't use it proactively without discipline. **Search before asking. Store before moving on.**

When a session starts, a "Recollect memory" block in your context names the project, shows its last session log and lists its recent notes and todos. Pass that project name with `-p` in the commands below.

## Where Things Go

Claude Code's auto memory and recollect keep different things. Store each fact in one place, never both.

| Auto memory | Recollect |
|-------------|-----------|
| How to work with the user: preferences, feedback, corrections | Decisions and their reasons |
| Conventions of this repository | Learnings: what turned out to be true, what failed |
| | Solved bugs: symptom, cause, fix |
| | Session logs (`/session-log`, compaction summaries) |

Auto memory is loaded into every session of one repository; recollect keeps the history and is searchable across projects.

## Retrieval: Search FIRST

**When you encounter a problem, error, or unfamiliar situation:**

1. Search memory BEFORE asking the user questions
2. Search memory BEFORE investigating the codebase
3. Only proceed to other approaches if memory search yields nothing relevant

```bash
recollect search "<error message or symptom>" -p <project> --json
recollect search "<words>" --json     # every project
recollect show <id>                   # one memory in full
```

Search matches words and meaning, so describe the symptom in your own words as well as quoting the error.

**No exceptions for urgency.** Production down? Search takes 2 seconds. Emergency? Search first anyway. The memory might contain the exact fix. Skipping search to "save time" often costs more time.

**Trigger phrases in your own thinking:**
- "I've never seen this before" → Search memory, you might have
- "Let me ask which..." → Search memory first
- "I need more context" → Search memory first
- "This is urgent" → Search memory, it's fast

## Storage: Store BEFORE Moving On

**When any of these happen, store immediately:**

| Event | Tags |
|-------|------|
| Decision made | `decision,<topic>` |
| Lesson learned | `learning,<topic>` |
| Bug solved | `bug,<symptom>` |
| Architecture choice | `architecture,<component>` |

Pass the content on stdin through a quoted heredoc, so quotes, backticks and `$` arrive unchanged:

```bash
recollect store -p <project> -T decision,auth <<'EOF'
Sessions expire after 8 hours of inactivity, not 24: the security review
asked for it, and the refresh token covers longer work.
EOF
```

Write each memory so it stands alone: what, why, and the context a reader months from now needs. Default to the project; store with `-p global` only knowledge that applies to every project, such as a tool's quirk or a pattern you use everywhere.

**Do not** say "I should store this" and then move on. Actually run the command.

**What counts as a decision?** If you discussed trade-offs, considered alternatives, or the choice affects future work → store it. Routine refactors (renaming a variable, extracting a method) with no discussion → skip.

## Projects

The session-start block names the project; pass it with `-p` to every command. To look at another project mid-session, run `recollect context -p <other>` (its last session and recent notes and todos); `recollect projects` lists all projects.

## Command Reference

| Task | Command |
|------|---------|
| Search one project | `recollect search "auth bug" -p <project> --json` |
| Search everywhere | `recollect search "auth bug" --json` |
| Read one memory in full | `recollect show 42` |
| Recent memories | `recollect list -p <project> --json` |
| Memories carrying all tags | `recollect list -T decision,auth --json` |
| A project's last session and notes | `recollect context -p <project>` |
| Store | `recollect store -p <project> -T tag1,tag2 <<'EOF'` … `EOF` |
| Store a todo | add `-t todo` (types: `note`, the default, `todo`, `session`) |
| Tag and project inventory | `recollect tags --json`, `recollect projects --json` |
| Delete | `recollect delete 42` |

`--json` output feeds `jq`, which keeps long results out of your context:

```bash
recollect search "deploy" -p <project> --json | jq -r '.[] | "\(.id): \(.content)"' | head -20
```

Warnings, such as a search that fell back to full text only, go to stderr; a non-zero exit code means the command failed.

## Red Flags - You're About to Fail

- Asking the user a question without searching memory first
- Saying "noted" or "I'll remember that" without running `recollect store`
- Debugging an error without checking if it was solved before
- Moving to the next task after a decision without storing it
- Skipping search because "it's urgent" or "production is down"
- Thinking "I already know how to fix this" without searching
- Storing the same fact in auto memory and in recollect

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "It's urgent, no time to search" | Search takes 2 seconds. Emergency is when memory helps most. |
| "I already know the fix" | Memory might have project-specific context you're missing. |
| "This is too trivial to store" | Did you discuss trade-offs? If yes, store it. |
| "Auto memory already has it" | Auto memory is for how to work here; decisions and fixes go to recollect, where every project can search them. |
