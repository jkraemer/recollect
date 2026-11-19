---
max_turns: 15
allowed_tools: [Bash, Read, Glob, Grep, Skill, Write, Edit]
---

Correction on staging: the database there is no longer reset on Sunday nights. Since the nightly data-cleanup job went in, it is reset every night, so test data only lives for a day. Nothing to implement.
