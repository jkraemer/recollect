---
name: session-log
description: Summarizes the current session and stores the summary in recollect as a session log, which the next session in the project starts with.
argument-hint: [extra notes to include]
disable-model-invocation: true
allowed-tools: Bash(recollect store *)
---

# Session Log

Summarize this session and store the summary in recollect; the next session in this project starts with it.

1. Go through the conversation for what was worked on, the decisions made and their reasons, the problems solved, the current state, and the next steps.

2. Write the summary in this format:

   ```
   Session: [Descriptive Title]
   Date: [Current UTC timestamp]

   ### Overview
   [2-3 sentences on what was accomplished]

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
   [What a future session needs to know to continue seamlessly]
   ```

3. Store it, passing the summary on stdin through a quoted heredoc so quotes, backticks and `$` arrive unchanged:

   ```bash
   recollect store -p <project> -t session -T <topic1>,<topic2> <<'EOF'
   <the summary>
   EOF
   ```

   `<project>` is the project named in the "Recollect memory" block in this session's context; use `-p global` for a session that belongs to no project. The tags name the session's topics.

4. Tell the user the memory ID from the output (`stored #<id>`).

$ARGUMENTS
