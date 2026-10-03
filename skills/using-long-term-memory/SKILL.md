---
name: using-long-term-memory
description: Searches and stores the long-term memory shared across sessions and projects with the recollect CLI. Use when an error, bug or question may have come up in an earlier session (search before answering or investigating), and right after a decision is made, something is learned or a bug is fixed (store it). Holds decisions with their reasons, learnings, solved bugs and session logs; these belong here rather than in auto memory.
---

# Using long-term memory

Recollect keeps decisions, learnings, solved bugs and session logs across sessions and projects, and finds them again by words and meaning. Search it before answering questions about earlier work, and store to it as soon as something worth keeping happens.

The "Recollect memory" block at the start of the session names the project and shows its last session log and recent notes. Pass that project name with `-p`. The commands below run in the shell with the Bash tool; loading this skill does not search or store anything by itself.

## What goes where

| Recollect | Claude Code's auto memory |
|-----------|---------------------------|
| Decisions and their reasons | How the user likes to work: preferences, feedback, corrections |
| Learnings: what turned out to be true, what failed | Conventions of this repository |
| Solved bugs: symptom, cause, fix | |
| Session logs (`/recollect:session-log`, compaction summaries) | |

Each fact goes to one place.

## Search first

Search when an error or symptom appears, when a question touches earlier work or decisions, and before asking the user something that may have been settled already:

```bash
recollect search "<symptom or topic, in your own words>" -p <project> --json
recollect search "<words>" --json        # all projects
recollect show <id>                      # one memory in full
```

Search matches words and meaning, so combine the error text with a description of the symptom. A search takes a second; it comes before investigating or asking, also when things are urgent.

## Store right away

Store when a decision is made (with its reason), when something is learned, and when a bug is solved (symptom, cause, fix). Tag by kind and topic: `decision`, `learning`, `bug` or `architecture`, plus the subject. Pass the content through a quoted heredoc so quotes, backticks and `$` arrive unchanged:

```bash
recollect store -p <project> -T decision,sessions <<'EOF'
Sessions expire after 8 hours of inactivity instead of 24: the security
review asked for it, and the refresh token covers longer work.
EOF
```

Write each memory so it stands on its own months later. Use `-p global` only for knowledge that applies to every project. A decision counts when alternatives were weighed or the choice shapes later work; a rename without discussion does not.

## Other commands

- `recollect list -p <project> --json`: recent memories; `-T decision,auth` keeps those carrying all the given tags
- `recollect context -p <other>`: another project's last session and recent notes
- `recollect projects`, `recollect tags`: what exists
- `recollect delete <id>`: removes a memory
- `-t todo` on `store` stores a todo (types: `note`, the default, `todo`, `session`)

Where `jq` is installed it can filter `--json` output. Warnings go to stderr; a non-zero exit means the command failed.

## Signs memory is being skipped

- Answering a question about earlier work, or debugging an error, without having searched
- Saying "noted" without running `recollect store`
- Writing a decision or a fix to auto memory instead of recollect
