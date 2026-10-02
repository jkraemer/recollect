---
description: Summarize this session and store it in long-term memory for a future session to resume from
argument-hint: [extra notes to include]
allowed-tools: Bash(recollect store *)
---

# Session Log

Create a session summary and store it in long-term memory; the next session in this project starts with it.

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

3. Store the summary with the recollect CLI, passing it on stdin through a quoted heredoc so quotes, backticks and `$` arrive unchanged:

   ```bash
   recollect store -p <project> -t session -T <topic1>,<topic2> <<'EOF'
   <the summary>
   EOF
   ```

   `<project>` is the project named in the "Recollect memory" block at the start of this session; use `-p global` for a session that belongs to no project. The tags name the session's topics.

4. Confirm storage to the user with the memory ID from the output (`stored #<id>`).

$ARGUMENTS
