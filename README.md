# Recollect

[![CI](https://github.com/jkraemer/recollect/actions/workflows/ci.yml/badge.svg)](https://github.com/jkraemer/recollect/actions/workflows/ci.yml)

A Ruby-based MCP (Model Context Protocol) server for persistent memory
management across Claude Code sessions.

## Overview

Recollect stores decisions, patterns, bugs, and learnings in SQLite databases
with FTS5 full-text search. It exposes memories via the MCP protocol over HTTP,
enabling AI coding assistants to maintain context across sessions.

## Features

- **MCP Protocol Support**: Standard MCP tools for storing and retrieving memories
- **Hybrid Search**: Combines BM25 full-text search with vector semantic search using **Reciprocal Rank Fusion (RRF)** for superior relevance
- **Smart Markdown Chunking**: Automatically splits large documents into semantic chunks (~700 words) with overlap for precise vector matching
- **Parent-Child Retrieval**: Transparently resolves chunk-level search matches back to the full original document
- **Recency Ranking**: Optional time-decay scoring to prefer newer memories
- **LLM-Powered (Optional)**: Query expansion and re-ranking using Anthropic Claude models
- **Project Isolation**: Separate database per project, plus a global database
- **REST API**: HTTP endpoints for the Web UI and CLI
- **Web Interface**: Browse and search memories in your browser
- **CLI Tool**: Command-line interface for quick memory operations

## Requirements

- Ruby >= 3.4.0
- SQLite3

### Optional: Vector Search

For semantic vector search (hybrid FTS5 + vector similarity):

- Python >= 3.8
- sqlite-vec extension (e.g., `pacman -S sqlite-vec` on Arch Linux)

### Optional: LLM Integration (Expansion & Re-ranking)

Recollect can use a remote LLM (like Anthropic's Claude 3 Haiku) to improve search quality through:
- **Query Expansion**: Generating alternative search terms to find conceptually related memories
- **Semantic Re-ranking**: Re-ordering the top results based on actual semantic relevance to your query

This is particularly powerful on slim hardware where running a large local embedding model isn't feasible.

```bash
export RECOLLECT_LLM_PROVIDER=anthropic
export ANTHROPIC_API_KEY=your_key_here
export RECOLLECT_ANTHROPIC_MODEL=claude-3-haiku-20240307
```

## Installation

Recollect ships through three channels: the gem carries the Ruby server and its
CLI (MCP tools, REST API, web UI), the Claude Code plugin carries the
agent-facing parts (skills, including `/recollect:session-log`, and hooks), and
GitHub Releases carry the Rust `recollect` binary. The plugin works through
that binary, which is replacing the Ruby server and needs no server running.

```bash
gem install recollect
recollect-server
```

Install the Rust binary with the install script. It picks the build for the
machine (Linux x86_64 or aarch64 with glibc 2.38 or newer, or macOS on Apple
Silicon), verifies its checksum and puts it in `~/.local/bin`, where it should
come before the gem's `recollect` command on PATH, which it replaces. Run it
again to upgrade:

```bash
curl -fsSL https://raw.githubusercontent.com/jkraemer/recollect/master/install.sh | sh
```

`RECOLLECT_VERSION=v0.1.0` pins a release and `RECOLLECT_INSTALL_DIR` picks
another directory. Other platforms are not supported for now: building
recollect from source there needs an ONNX Runtime built for the platform.

The x86_64 build needs a CPU with AVX2 (Intel since Haswell, 2013; AMD since
2015), and the install script checks for it. A virtual machine has to pass
AVX2 through to the guest: with QEMU, Proxmox or libvirt, choose the CPU type
`host`. Generic types such as `qemu64`, `kvm64` or `x86-64-v2-AES` hide it,
and recollect stops with `Illegal instruction` there.

Then, in Claude Code:

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

### Optional: Set Up Vector Search

Semantic vector search needs Python with `sentence-transformers`:

```bash
python3 -m venv ~/.recollect/venv
~/.recollect/venv/bin/pip install sentence-transformers
```

Then start the server with vectors enabled:

```bash
RECOLLECT_ENABLE_VECTORS=true RECOLLECT_PYTHON=~/.recollect/venv/bin/python3 recollect-server
```

From a checkout, a `.venv` in the project root is picked up automatically and
`RECOLLECT_PYTHON` is not needed:

```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
RECOLLECT_ENABLE_VECTORS=true ./bin/server
```

### Optional: Enable Recency Ranking

Recency ranking applies time-decay scoring to search results, preferring newer memories
over older ones with similar relevance. This is useful when recent context is more
valuable than historical information.

```bash
RECOLLECT_RECENCY_AGING_FACTOR=0.5 RECOLLECT_RECENCY_HALF_LIFE_DAYS=30 recollect-server
```

- **Aging Factor** (0.0-1.0): How much recency affects ranking. 0=disabled, 1=full effect.
- **Half-Life Days**: Days until a memory's recency score decays to 50%.

With `aging_factor=0.5` and `half_life_days=30`, a 30-day-old memory keeps 75% of its
relevance score, while a brand-new memory keeps 100%.

## Syncing between machines

Machines that run the Rust binary can share their memories directly with
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

## Usage

### Start the Server

```bash
recollect-server
```

The server runs at `http://localhost:7326` by default. To keep it running across
reboots, see [Running as a systemd Service](#running-as-a-systemd-service).

### Configure Claude Code

The plugin does not use the server. To give Claude the Ruby server's MCP tools,
add to your MCP configuration:

```json
{
  "mcpServers": {
    "recollect": {
      "type": "http",
      "url": "http://localhost:7326/mcp"
    }
  }
}
```

### Project Naming

Recollect stores memories per-project. With the Claude Code plugin, the
session-start hook names the project (see [Installation](#installation)) and
the agent passes that name on. For the MCP tools and other agents, ensure
consistent naming across sessions by adding an instruction to your project's
agent instructions (AGENTS.md, CLAUDE.md):

> When storing or recalling memories, refer to this project as "myproject"

Without this, different sessions might use inconsistent names (directory basename,
repo name, etc.) which fragments memories across separate databases.

### Claude Code Skill

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

### CLI Commands

```bash
# Check server status
recollect status

# Store a memory
recollect store "We decided to use Puma for threading" -p myproject -t decision

# Search memories
recollect search "threading"

# List recent memories
recollect list -p myproject

# List all projects
recollect projects
```

### Web UI

Open `http://localhost:7326` in your browser to browse and search memories.

## MCP Tools

| Tool | Description |
|------|-------------|
| `store_memory` | Store a memory with content, type, tags, and project |
| `search_memory` | Full-text search across memories |
| `get_context` | Get comprehensive context for a project |
| `list_projects` | List all projects with stored memories |
| `delete_memory` | Delete a specific memory by ID |

All tools declare an `outputSchema` and return `structuredContent` alongside
the JSON text content, so typed clients get validated results while text-only
clients keep working.

### Memory Types

- `note` (default) - General information, facts, context
- `todo` - Action items, tasks, reminders
- `session` - Session summaries and handoff notes

For semantic categorization (decisions, patterns, bugs, learnings), use **tags** instead of memory types. This provides more flexible filtering and allows memories to have multiple categories.

## MCP Resources

Project memory is browsable as resources with markdown bodies:

| URI | Contents |
|-----|----------|
| `recollect://project/{name}` | Listable, one per project (plus `global`): last session log and recent notes/todos |
| `recollect://project/{project}/memory/{id}` | Template: a single memory by project and id |

## MCP Prompts

Prompts are reusable templates that guide AI assistants through common workflows.

| Prompt | Description |
|--------|-------------|
| `session_log` | Create a structured session summary and store it for future retrieval |
| `resume_session` | Resume work using the last session log and recent memories |

### Session Workflow

At the end of a session, use `session_log` to capture what was worked on, decisions made,
problems solved, and next steps. This creates a "session" memory type.

When starting a new session, use `resume_session` to retrieve the last session log and
recent memories, providing context for continuing where you left off.

#### resume_session Details

The `resume_session` prompt takes an optional `project` argument:

- **With project**: Retrieves the last session log and 10 most recent memories (notes/todos)
  for that project, then asks the AI to summarize and propose next steps
- **Without project**: Provides guidance for the AI to determine the project from context
  (working directory, conversation, or by calling `get_context` without parameters)

This makes it easy to pick up where you left off, even if you don't remember the exact
project name or what you were working on.

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `7326` | Server port |
| `RECOLLECT_URL` | `http://localhost:7326` | CLI base URL |
| `RECOLLECT_ENABLE_VECTORS` | `false` | Enable vector search |
| `RECOLLECT_MAX_VECTOR_DISTANCE` | `1.0` | Max cosine distance (0-2) for vector results |
| `RECOLLECT_PYTHON` | `.venv/bin/python3`, else `python3` | Python interpreter running the embedding model |
| `RECOLLECT_SQLITE_VEC_PATH` | (auto-detect) | Path to the sqlite-vec extension, checked before built-in locations |
| `RECOLLECT_LOG_WIREDUMPS` | `false` | Enable debug logging |
| `RECOLLECT_RECENCY_AGING_FACTOR` | `0.0` | Recency ranking strength (0.0-1.0, 0=disabled) |
| `RECOLLECT_RECENCY_HALF_LIFE_DAYS` | `30.0` | Days until memory relevance decays to 50% |
| `RECOLLECT_LLM_PROVIDER` | `none` | LLM provider (`none`, `anthropic`) |
| `ANTHROPIC_API_KEY` | | API key for Anthropic provider |
| `RECOLLECT_ANTHROPIC_MODEL` | `claude-3-haiku-20240307` | Model to use for Anthropic |
| `WEB_CONCURRENCY` | `1` | Puma worker processes |
| `PUMA_MAX_THREADS` | `5` | Threads per worker |

## Running as a systemd Service

See [docs/systemd/README.md](docs/systemd/README.md) for setup instructions to run Recollect as a user systemd service.

## Development

```bash
git clone https://github.com/jkraemer/recollect.git
cd recollect
bundle install

# Run the server and CLI from the working copy
./bin/server
./bin/recollect status

# Run tests
bundle exec rake test

# Run single test file
bundle exec ruby -Itest test/recollect/database_test.rb

# Lint
bundle exec rubocop
```

`bin/server` and `bin/recollect` are thin wrappers that load the same code the
gem installs as `recollect-server` and `recollect`.

### Packaging layout

The repository is a gem, a Claude Code plugin marketplace and the source of the recollect binary:

| Path | Channel | Contents |
|------|---------|----------|
| `recollect.gemspec`, `exe/`, `lib/`, `config/`, `public/` | gem | server and CLI |
| `.claude-plugin/plugin.json` | plugin | plugin manifest |
| `.claude-plugin/marketplace.json` | plugin | catalog, so this repo can be added as a marketplace |
| `skills/`, `hooks/` | plugin | skills (memory discipline, `/recollect:session-log`), hooks running the `recollect` binary |
| `Cargo.toml`, `src/`, `install.sh` | binary (GitHub Releases) | the Rust `recollect` CLI and its installer |

The plugin carries its own version: `tests/plugin.rs` checks its manifests,
hooks and skills, `test/packaging_test.rb` checks the gem. To try the
plugin without publishing, load the checkout for one session with
`claude --plugin-dir /path/to/recollect`, or add it as a local marketplace:

```
/plugin marketplace add /path/to/recollect
/plugin install recollect@recollect
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   Sinatra/Puma Server                   │
├─────────────────────────────────────────────────────────┤
│  POST /mcp         → MCP protocol endpoint              │
│  GET/POST /api/*   → REST API                           │
│  GET /             → Web UI                             │
└─────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────┐
│              SQLite + FTS5 (per-project)                │
├─────────────────────────────────────────────────────────┤
│  ~/.recollect/global.db        → Cross-project memories │
│  ~/.recollect/projects/*.db    → Project-specific       │
└─────────────────────────────────────────────────────────┘
```

## License

GPL-3.0-or-later
