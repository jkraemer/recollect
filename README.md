# Recollect

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

Recollect ships through two channels: the gem carries the server and the CLI,
the Claude Code plugin carries the agent-facing parts (skill, `/session-log`
command, MCP wiring). Install both.

```bash
gem install recollect
recollect-server
```

Then, in Claude Code:

```
/plugin marketplace add jkraemer/recollect
/plugin install recollect@recollect
```

The plugin points Claude at `http://localhost:7326/mcp`. If you moved the server
to another port, configure the MCP server by hand instead - see
[Configure Claude Code](#configure-claude-code).

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

## Usage

### Start the Server

```bash
recollect-server
```

The server runs at `http://localhost:7326` by default. To keep it running across
reboots, see [Running as a systemd Service](#running-as-a-systemd-service).

### Configure Claude Code

Installing the plugin wires this up for you. To do it by hand - or to point Claude
at a server on a different host or port - add to your MCP configuration:

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

Recollect stores memories per-project. To ensure consistent naming across sessions,
add an instruction to your project's agent instructions (AGENTS.md, CLAUDE.md):

> When storing or recalling memories, refer to this project as "myproject"

Without this, different sessions might use inconsistent names (directory basename,
repo name, etc.) which fragments memories across separate databases.

### Claude Code Skill

Memory tools only help if the agent reaches for them. The
`using-long-term-memory` skill enforces two disciplines:

1. **Search before asking** - When encountering problems or unfamiliar situations,
   search memory before asking the user or investigating the codebase
2. **Store before moving on** - When decisions are made, lessons learned, or bugs
   solved, store them immediately with appropriate tags

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

## MCP Resources

Project memory is browsable as resources with markdown bodies:

| URI | Contents |
|-----|----------|
| `recollect://project/{name}` | Listable, one per project (plus `global`): last session log and recent notes/todos |
| `recollect://project/{project}/memory/{id}` | Template: a single memory by project and id |

### Memory Types

- `note` (default) - General information, facts, context
- `todo` - Action items, tasks, reminders
- `session` - Session summaries and handoff notes

For semantic categorization (decisions, patterns, bugs, learnings), use **tags** instead of memory types. This provides more flexible filtering and allows memories to have multiple categories.

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

The repository is both a gem and a Claude Code plugin marketplace:

| Path | Channel | Contents |
|------|---------|----------|
| `recollect.gemspec`, `exe/`, `lib/`, `config/`, `public/` | gem | server and CLI |
| `.claude-plugin/plugin.json` | plugin | plugin manifest |
| `.claude-plugin/marketplace.json` | plugin | catalog, so this repo can be added as a marketplace |
| `skills/`, `commands/`, `.mcp.json` | plugin | skill, slash command, MCP wiring |

Both carry the same version number; `test/packaging_test.rb` fails if they drift
apart. To try the plugin without publishing, add the checkout as a local
marketplace:

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
