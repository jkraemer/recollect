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

3. Store the summary using the store_memory tool:
   - memory_type: "session"
   - tags: [relevant topic tags]
   - project: current project name (or omit for cross-project sessions)

4. Confirm storage to the user with the memory ID.

$ARGUMENTS
