# Recollect

A Ruby-based MCP (Model Context Protocol) server for persistent memory
management across Claude Code sessions.

## Overview

Recollect stores decisions, patterns, bugs, and learnings in SQLite databases
with FTS5 full-text search. It exposes memories via the MCP protocol over HTTP,
enabling AI coding assistants to maintain context across sessions.

## Features

- **MCP Protocol Support**: Standard MCP tools for storing and retrieving memories
- **Full-Text Search**: SQLite FTS5 for fast, relevant search results
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

## Installation

```bash
git clone <repo-url>
cd ruby-mcp-memory
bundle install
```

### Optional: Set Up Vector Search

To enable semantic vector search, create a Python virtual environment and install dependencies:

```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

Then set the environment variable when starting the server:

```bash
RECOLLECT_ENABLE_VECTORS=true ./bin/server
```

## Usage

### Start the Server

```bash
./bin/server
```

The server runs at `http://localhost:7326` by default.

### Configure Claude Code

Add to your Claude Code MCP configuration:

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
add an instruction to your project's CLAUDE.md:

> When storing or recalling memories, refer to this project as "myproject"

Without this, different sessions might use inconsistent names (directory basename,
repo name, etc.) which fragments memories across separate databases.

### Claude Code Skill & Command

For effective memory usage, install the `using-long-term-memory` skill in your
`~/.claude/skills/` directory. This skill enforces two core disciplines:

1. **Search before asking** - When encountering problems or unfamiliar situations,
   search memory before asking the user or investigating the codebase
2. **Store before moving on** - When decisions are made, lessons learned, or bugs
   solved, store them immediately with appropriate tags

See [docs/using-long-term-memory-skill.md](docs/using-long-term-memory-skill.md) for the full skill.

Additionally, the `/recollect:session-log` command creates structured session
summaries that capture what was worked on, key decisions, problems solved, and
next steps. Run it at the end of a session to preserve context for future work.

### CLI Commands

```bash
# Check server status
./bin/recollect status

# Store a memory
./bin/recollect store "We decided to use Puma for threading" -p myproject -t decision

# Search memories
./bin/recollect search "threading"

# List recent memories
./bin/recollect list -p myproject

# List all projects
./bin/recollect projects
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

### Memory Types

- `note` (default) - General information, facts, context
- `todo` - Action items, tasks, reminders
- `session` - Session summaries and handoff notes

For semantic categorization (decisions, patterns, bugs, learnings), use **tags** instead of memory types. This provides more flexible filtering and allows memories to have multiple categories.

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `7326` | Server port |
| `RECOLLECT_URL` | `http://localhost:7326` | CLI base URL |
| `RECOLLECT_ENABLE_VECTORS` | `false` | Enable vector search |
| `RECOLLECT_MAX_VECTOR_DISTANCE` | `1.0` | Max cosine distance (0-2) for vector results |
| `RECOLLECT_LOG_WIREDUMPS` | `false` | Enable debug logging |
| `WEB_CONCURRENCY` | `1` | Puma worker processes |
| `PUMA_MAX_THREADS` | `5` | Threads per worker |

## Running as a systemd Service

See [docs/systemd/README.md](docs/systemd/README.md) for setup instructions to run Recollect as a user systemd service.

## Development

```bash
# Run tests
bundle exec rake test

# Run single test file
bundle exec ruby -Itest test/recollect/database_test.rb

# Lint
bundle exec rubocop
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
