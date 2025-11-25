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
ENABLE_VECTORS=true ./bin/server
```

## Usage

### Start the Server

```bash
./bin/server
```

The server runs at `http://localhost:8080` by default.

### Configure Claude Code

Add to your Claude Code MCP configuration:

```json
{
  "mcpServers": {
    "recollect": {
      "type": "http",
      "url": "http://localhost:8080/mcp"
    }
  }
}
```

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

Open `http://localhost:8080` in your browser to browse and search memories.

## MCP Tools

| Tool | Description |
|------|-------------|
| `store_memory` | Store a memory with content, type, tags, and project |
| `search_memory` | Full-text search across memories |
| `get_context` | Get comprehensive context for a project |
| `list_projects` | List all projects with stored memories |
| `delete_memory` | Delete a specific memory by ID |

### Memory Types

- `note` (default)
- `decision`
- `pattern`
- `bug`
- `learning`

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `8080` | Server port |
| `RECOLLECT_URL` | `http://localhost:8080` | CLI base URL |
| `WEB_CONCURRENCY` | `2` | Puma worker processes |
| `PUMA_MAX_THREADS` | `5` | Threads per worker |

## Running as a systemd Service

To run Recollect automatically on login, create a user systemd service.

First, edit `bin/start-service` and uncomment the line for your Ruby version manager (rbenv, rvm, or asdf).

Then create the systemd unit:

```bash
mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/recollect.service << 'EOF'
[Unit]
Description=Recollect MCP Memory Server
After=network.target

[Service]
Type=simple
ExecStart=/path/to/recollect/bin/start-service
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF
```

Update the paths to match your installation, then enable and start:

```bash
systemctl --user daemon-reload
systemctl --user enable recollect
systemctl --user start recollect

# Check status
systemctl --user status recollect

# View logs
journalctl --user -u recollect -f
```

To ensure the service runs even when not logged in:

```bash
loginctl enable-linger $USER
```

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
