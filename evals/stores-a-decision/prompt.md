---
max_turns: 15
allowed_tools: [Bash, Read, Glob, Grep, Skill]
---

Decision on invoice numbering: numbers restart at 1 every calendar year, prefixed with the year (2026-0001). We chose this because the tax office requires a gapless sequence per fiscal year, and a global counter would leave gaps once we archive old years. Nothing to implement yet.
