# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

When storing or recalling memories, refer to this project as "recollect".

## Project Overview

**Recollect** is a Ruby-based HTTP MCP (Model Context Protocol) server for persistent memory management. It stores memories in SQLite databases with FTS5 full-text search, accessible via MCP protocol over HTTP.

## Commands

```bash
# Run tests
bundle exec rake test

# Run a single test file
bundle exec ruby -Itest test/recollect/database_test.rb

# Run specific test method
bundle exec ruby -Itest test/recollect/database_test.rb -n test_store_returns_id

# Lint
bundle exec rubocop

# Start server (development)
./bin/server
# Or: bundle exec puma -C config/puma.rb

# CLI commands (requires running server)
./bin/recollect status
./bin/recollect store "content" -p project -t decision
./bin/recollect search "query"
./bin/recollect list
./bin/recollect projects
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   Sinatra/Puma Server                   │
├─────────────────────────────────────────────────────────┤
│  POST /mcp         → MCP::Server#handle_json(body)      │
│  GET/POST /api/*   → REST endpoints for Web UI + CLI    │
│  GET /             → Static Web UI files                │
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

### Key Components

- **HTTPServer** (`lib/recollect/http_server.rb`): Sinatra app handling MCP endpoint, REST API, and static files
- **MCPServer** (`lib/recollect/mcp_server.rb`): Factory building MCP::Server with all tools
- **DatabaseManager** (`lib/recollect/database_manager.rb`): Multi-database coordination with lazy initialization
- **Database** (`lib/recollect/database.rb`): SQLite wrapper with FTS5 search
- **Tools** (`lib/recollect/tools/`): MCP tool implementations (store, search, get_context, list_projects, delete)

### Design Decisions

- **HTTP-only transport**: No stdio; single Puma server simplifies SQLite concurrency
- **MCP via handle_json**: MCP protocol exposed at `/mcp` endpoint
- **Project isolation**: Separate database per project, plus global database
- **Vector search**: Optional hybrid FTS5 + vector similarity search via sqlite-vec extension

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `7326` | Server port |
| `RECOLLECT_URL` | `http://localhost:7326` | CLI base URL |
| `RECOLLECT_ENABLE_VECTORS` | `false` | Enable vector search |
| `RECOLLECT_MAX_VECTOR_DISTANCE` | `1.0` | Max cosine distance (0-2) for vector results |
| `RECOLLECT_LOG_WIREDUMPS` | `false` | Enable debug logging |
| `RECOLLECT_RECENCY_AGING_FACTOR` | `0.0` | Recency ranking strength (0.0-1.0, 0=disabled) |
| `RECOLLECT_RECENCY_HALF_LIFE_DAYS` | `30.0` | Days until memory relevance decays to 50% |

## Before Committing

Run rubocop to detect and fix any style offenses:

```bash
bundle exec rake rubocop
```

Run test coverage and ensure it hasn't degraded:

```bash
bundle exec rake coverage
```

Degrading test coverage is strongly discouraged. If coverage drops, add tests for uncovered code before committing.

## Testing

Tests use `test/tmp/test_data` for isolated database files (cleaned between tests). Test helper sets `RACK_ENV=test` and provides `Recollect::TestCase` base class with Rack::Test methods.

## MCP Configuration for Claude Code

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
