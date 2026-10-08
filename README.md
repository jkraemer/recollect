# Recollect

[![CI](https://github.com/jkraemer/recollect/actions/workflows/ci.yml/badge.svg)](https://github.com/jkraemer/recollect/actions/workflows/ci.yml)

Persistent, searchable memory for coding agents: one binary that keeps
decisions, learnings, solved bugs and session logs in a local SQLite database
and finds them again by words and by meaning.

## Overview

Recollect is a command-line tool. An agent, or you, stores a memory with
`recollect store` and finds it again with `recollect search`: in a later
session, in another project or on another machine. There is no server to run
and no account to create. The database is one file in `~/.recollect`, and the
embedding model runs inside the binary.

## Features

- **One binary**: SQLite with FTS5, the sqlite-vec extension and the embedding runtime are compiled in; there is nothing else to install
- **Hybrid search**: BM25 full-text search and vector similarity, merged with reciprocal rank fusion
- **Local embeddings**: computed in-process with `bge-small-en-v1.5` (English); no memory is sent to an embedding service
- **Projects**: a memory belongs to a project or to none (global), and a search covers one project or all of them
- **Immutable memories**: a memory is stored and at most deleted, never edited, so sync cannot conflict
- **Sync between machines**: directly from machine to machine, encrypted, without a server in between
- **Claude Code plugin**: hooks that put a project's memory into each session, and skills that make the agent search and store
- **Made for agents**: content from stdin, `--json` output, one-line errors and a non-zero exit status on failure
- **Recency ranking**: optional time decay that prefers newer memories

## Installation

Recollect ships through two channels: GitHub Releases carry the `recollect`
binary, and the Claude Code plugin carries the agent-facing parts (skills,
including `/recollect:session-log`, and hooks). The plugin works through the
binary.

Install the binary with the install script. It picks the build for the
machine (Linux x86_64 or aarch64 with glibc 2.38 or newer, or macOS on Apple
Silicon), verifies its checksum and puts it in `~/.local/bin`. Run it again
to upgrade, or use recollect update (below):

```bash
curl -fsSL https://raw.githubusercontent.com/jkraemer/recollect/master/install.sh | sh
```

`RECOLLECT_VERSION=v0.1.0` pins a release and `RECOLLECT_INSTALL_DIR` picks
another directory. Other platforms are not supported for now: building
recollect from source there needs an ONNX Runtime built for the platform.

The x86_64 build needs a CPU with AVX2, and the install script checks for it.
Most desktop and server CPUs since about 2015 have it; low-power Atom-class
Pentium and Celeron models do not. A virtual machine has to pass AVX2 through
to the guest: choose the CPU type `host` in QEMU or Proxmox, or the mode
`host-passthrough` in libvirt. Generic types such as `qemu64`, `kvm64` or
`x86-64-v2-AES` hide it, and recollect stops with `Illegal instruction` there.

The first `store` or `search` downloads the embedding model (about 65 MB)
from Hugging Face into `~/.recollect/models`. If that fails, the memory is
stored all the same and search matches words only; `recollect reindex` embeds
what is pending once the model is there.

Once installed, recollect upgrades itself:

```bash
recollect update           # install the latest release over the running binary
recollect update --check   # only say whether there is a newer one
```

When a session starts, recollect looks up the latest release (at most once a
day, giving up after two seconds) and, if there is a newer one, tells the
agent to mention it. The agent is told not to run the update: that stays
your decision. To switch the lookup and the notice off, put this into
`~/.recollect/config.toml`:

```toml
[update]
check = false
```

### Claude Code plugin

In Claude Code:

```
/plugin marketplace add jkraemer/recollect
/plugin install recollect@recollect
```

When a session starts, and after `/clear`, the plugin's hook puts the current
project's memory into context: its last session log and an index of its recent
notes and todos. When Claude Code compacts the conversation, the other hook
stores the compaction summary as a session memory, and the project name and
the commands come back into context. The project is the git
repository's directory name (the main repository's, in a worktree); a
`.recollect-project` file holding a name overrides it for its directory and
everything below it in the same repository, and a worktree also uses the one
at its main checkout's root. To let the agent run the CLI without asking each
time, add
`Bash(recollect *)` to `permissions.allow` in your Claude Code settings.

To run from a checkout instead, see [Development](#development).

### Skill

Memory only helps if the agent reaches for it. The `using-long-term-memory`
skill enforces three disciplines, through the `recollect` CLI:

1. **Search before asking** - When encountering problems or unfamiliar situations,
   search memory before asking the user or investigating the codebase
2. **Store before moving on** - When decisions are made, lessons learned, or bugs
   solved, store them immediately with appropriate tags
3. **One place per fact** - How to work with the user and repository conventions
   go to Claude Code's auto memory; decisions, learnings, solved bugs and session
   logs go to recollect

The plugin installs it. Agents other than Claude Code can pick it up from
[skills/using-long-term-memory/SKILL.md](skills/using-long-term-memory/SKILL.md),
which follows the [Agent Skills](https://agentskills.io) `skills/*/SKILL.md`
convention:

```bash
npx skills add jkraemer/recollect
# or
gh skill install jkraemer/recollect
```

## Usage

```bash
# Store: the content is the argument, or comes from stdin
recollect store "The staging server runs Debian 13" -p myproject
recollect store -p myproject -T decision,sessions <<'EOF'
Sessions expire after 8 hours of inactivity instead of 24: the security
review asked for it, and the refresh token covers longer work.
EOF

# Find
recollect search "session expiry" -p myproject   # by words and meaning
recollect search "session expiry"                # in all projects
recollect list -p myproject -T decision --since 2026-01-01
recollect show 42                                # one memory in full
recollect context -p myproject                   # the last session, recent notes and todos

# Look around
recollect projects
recollect tags -p myproject
recollect status                                 # storage location, counts, vector health

# Remove
recollect delete 42
```

`store`, `search`, `list`, `show`, `context`, `projects`, `tags` and `status`
print JSON with `--json`. `search` and `list` take filters: `-t` for memory
types, `-T` for tags a memory must carry (all of them), and `--since` and
`--until` for the time it was stored. `recollect <command> --help` lists
every option.

### Projects

`-p` names the project. `store` without `-p`, or with `-p global`, stores a
memory that belongs to no project. `search`, `list` and `tags` without `-p`
cover every project, and `-p global` selects the memories without one.

With the Claude Code plugin, the session-start hook names the project (see
[Claude Code plugin](#claude-code-plugin)) and the agent passes that name on.
For other agents, keep the name the same across sessions with a line in the
project's agent instructions (AGENTS.md):

> When storing or recalling memories, refer to this project as "myproject"

Without it, sessions may pick different names (the directory, the repository)
and split one project's memories between them.

### Memory Types

- `note` (default) - General information, facts, context
- `todo` - Action items, tasks, reminders
- `session` - Session summaries and handoff notes

For semantic categorization (decisions, patterns, bugs, learnings), use **tags** instead of memory types. This provides more flexible filtering and allows memories to have multiple categories.

### Search

A search runs two queries and merges their rankings: one over the words
(SQLite FTS5; common English words such as "the" or "how" are left out unless
they are quoted) and one over the meaning (the query's embedding against the
memories' embeddings). Long memories are embedded in several chunks and found
as a whole. Memories that have no vectors yet are still found by their words.

### Deleting

Memories never change once stored. `recollect delete` removes a memory's
text and tags and keeps an empty record of the deletion, so that sync can
pass the deletion on to the other machines.

## Syncing between machines

Machines can share their memories directly with
each other: no server in between, and any network on which one machine can
reach the other will do (a LAN, a VPN, a forwarded port). The traffic is
encrypted, and each machine only talks to the machines it was paired with.

On each machine, run the sync daemon. As a systemd user service:

```bash
mkdir -p ~/.config/systemd/user
curl -fsSL -o ~/.config/systemd/user/recollect-serve.service \
  https://raw.githubusercontent.com/jkraemer/recollect/master/docs/systemd/recollect-serve.service
systemctl --user daemon-reload
systemctl --user enable --now recollect-serve
```

To keep the daemon running while you are logged out, enable lingering once:
`loginctl enable-linger`.

The daemon needs no restart after an upgrade: when it finds another file
where its binary was, it finishes the rounds in progress (waiting up to five
minutes for them) and starts again as the new version. A daemon started by
recollect 0.2.0 or older cannot do that yet: restart it once after upgrading
from such a version (`systemctl --user restart recollect-serve`).

It listens on port 7327. If a firewall blocks incoming connections (Fedora's
does by default), open the port on at least one of the two machines, for
example `sudo firewall-cmd --permanent --add-port=7327/tcp && sudo firewall-cmd --reload`,
or put the network interface the machines share (a VPN's, for example) into a
trusted zone.

Pair two machines once. On a machine the other one can reach (one whose port
is open):

```bash
recollect pair
```

This prints a `recollect join …` command. Run it on the other machine
within ten minutes. Treat that command like a password until it is used:
whoever runs it first, within those ten minutes, becomes a peer and receives
every memory. From then on the two are equals: each knows the other's name,
key and address, each syncs with the other when it starts and every five
minutes, and one working direction is enough. `recollect sync` runs a round
right away, and `recollect peer list` shows every peer with its last sync
and, if this machine's own last round with it failed, why. A machine that
cannot reach a peer keeps showing that failure there, even while the peer's
own rounds keep the two in sync. `recollect sync` on such a machine reports
that failure and exits with status 1 for the same reason.

`recollect pair` and `recollect join` assume a machine is reached under its
host name; where that does not resolve on the other machine, pass
`--address <host-or-ip>:7327` to both, or correct a peer later with
`recollect peer address <name> <host:port>`. All peers of a machine need
different names, and a machine's name is its host name up to the first dot
unless it sets one. Where two would clash, set a name on one of them in
`~/.recollect/config.toml`.

```toml
[sync]
name = "laptop"            # default: the host name up to its first dot
listen = "0.0.0.0:7327"    # address and port recollect serve listens on
interval_seconds = 300     # time between the daemon's rounds
```

Memories never change once stored, so sync cannot conflict: after a round
both machines hold every memory and every deletion of both. Embeddings are
not sent; each machine computes them for what it receives. Machines must run
releases that speak the same sync protocol; if they do not, `recollect peer
list` says which side to upgrade.

## Configuration

Everything is optional. Recollect reads `config.toml` in its data directory;
a key it does not know is an error. The `[sync]` and `[update]` tables are
described above, these two tune search:

```toml
[search]
max_vector_distance = 0.375   # cosine distance (0 to 2) up to which a memory matches by meaning

[recency]
aging_factor = 0.0            # 0 switches recency ranking off, 1 applies the full decay
half_life_days = 30.0         # the age at which the decay has reached one half
```

Recency ranking scales each result's score by its memory's age, so that of
two similarly relevant memories the newer one comes first. With
`aging_factor = 0.5` and `half_life_days = 30`, a 30-day-old memory keeps 75%
of its score and a new one keeps all of it.

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data directory: the database, `config.toml` and this machine's sync key |
| `RECOLLECT_MODEL_DIR` | `<data dir>/models` | Where the embedding model is kept |

## Coming from the Ruby server

Recollect began as a Ruby MCP server with a web UI. That version is retired;
its code is in this repository up to the tag `v0.3.0`. To bring its memories
over, point `migrate-from-ruby` at the Ruby server's data directory, the one
that holds `global.db` and `projects/` (`~/.recollect` by default):

```bash
recollect migrate-from-ruby ~/.recollect
recollect migrate-from-ruby ~/.recollect --rename my_proj=my-proj   # merge two spellings of one project
```

The Ruby files are only read. The command can be run again: it skips the
memories that are already there and deletes those the Ruby server has deleted
since. Both versions can use `~/.recollect`, their files have different
names.

## Development

```bash
git clone https://github.com/jkraemer/recollect.git
cd recollect

# Run the tests; the first run downloads the embedding model to .model-cache/
cargo test

# Format and lint
cargo fmt --check && cargo clippy --all-targets -- -D warnings

# Run the CLI from the working copy
cargo run -- store -p myproject -T decision "Memory content"
cargo run -- search "query" --json
```

`rust-toolchain.toml` pins the toolchain. `cargo run` and `cargo test` keep
their data in `target/test-data`, never in `~/.recollect`, and a development
build started directly (`target/debug/recollect`) refuses to run without
`RECOLLECT_DATA_DIR`.

### Packaging layout

The repository is the source of the recollect binary and a Claude Code plugin marketplace:

| Path | Channel | Contents |
|------|---------|----------|
| `Cargo.toml`, `src/`, `install.sh` | binary (GitHub Releases) | the `recollect` CLI and its installer |
| `.claude-plugin/plugin.json` | plugin | plugin manifest |
| `.claude-plugin/marketplace.json` | plugin | catalog, so this repo can be added as a marketplace |
| `skills/`, `hooks/` | plugin | skills (memory discipline, `/recollect:session-log`), hooks running the `recollect` binary |

The plugin carries its own version: `tests/plugin.rs` checks its manifests,
hooks and skills. To try the
plugin without publishing, load the checkout for one session with
`claude --plugin-dir /path/to/recollect`, or add it as a local marketplace:

```
/plugin marketplace add /path/to/recollect
/plugin install recollect@recollect
```

## License

GPL-3.0-or-later
