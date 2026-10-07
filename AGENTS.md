# AGENTS.md

Guidance for AI coding agents working in this repository. `CLAUDE.md` and `GEMINI.md`
are symlinks to this file.

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

# Build the gem / check its packaging
gem build recollect.gemspec
bundle exec ruby -Itest test/packaging_test.rb

# CLI commands (requires running server)
./bin/recollect status
./bin/recollect store "content" -p project -t decision
./bin/recollect search "query"
./bin/recollect list
./bin/recollect projects
```

### Rust rewrite (in progress)

The Rust crate at the repository root (`Cargo.toml`, `src/`, `tests/`) is the
local-first CLI replacing the Ruby server; see
`docs/superpowers/specs/2026-09-29-rust-core-cli-design.md` (untracked
working doc). It is excluded from the gem.

```bash
cargo test                                   # all tests; the first run downloads the model to .model-cache/
cargo test --test cli                        # end-to-end tests of the binary
cargo test --test plugin                     # the Claude Code plugin's manifests, hooks and skills
cargo test --test sync                       # sync end to end: daemons and CLI on local sockets
cargo test --test update                     # the update notice and `recollect update` against a fake release site
evals/run.sh --model opus                    # the plugin's behaviour evals (claude plugin eval; paid model calls)
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo llvm-cov --fail-under-lines 80         # coverage floor enforced in CI
cargo run -- store -p myproj -T decision <<'EOF'
Memory content from stdin
EOF
cargo run -- search "query" --json
```

Data lives in `$RECOLLECT_DATA_DIR/memories.db` (default `~/.recollect`); the
embedding model is cached in `$RECOLLECT_MODEL_DIR` (default
`<data dir>/models`). fastembed lets `HF_HOME` override the model directory, so
keep `HF_HOME` unset: the tests expect it to be.

`cargo run` and `cargo test` use `target/test-data` as the data directory
unless `RECOLLECT_DATA_DIR` is set: `.cargo/config.toml` sets it for
everything cargo starts, so a development build cannot open `~/.recollect` by
accident and migrate the real database to its own schema version, which an
older installed `recollect` then refuses to open. A development build started
directly (`target/debug/recollect …`) does not get that default and refuses
to start without `RECOLLECT_DATA_DIR`; only a release build falls back to
`~/.recollect`. The embedding model comes from `.model-cache` the same way.

The Claude Code plugin's hooks (`hooks/hooks.json`) run `recollect hook
session-start` and `recollect hook post-compact` with Claude Code's hook
input on stdin; that corner of the CLI is `src/hook.rs`, with project
detection (a `.recollect-project` file, else the git repository's directory
name) in `src/detect.rs`.

`evals/` holds `claude plugin eval` cases for the skill: whether an agent
searches recollect before answering, stores decisions in the right project,
and keeps working preferences in auto memory instead. `evals/run.sh` stages the plugin (cargo's hard links in `target/`
make `claude plugin eval .` refuse the repository) and runs them against the
`recollect` on PATH, so install the build under test first (`cargo install
--path . --locked --root ~/.local`). Bash in eval runs needs `bubblewrap`
and `socat`.

Releases: set `version` in `Cargo.toml`, run `cargo build` so `Cargo.lock`
follows, commit, tag `vX.Y.Z` and push master and the tag to `gh` (master to
`origin` as well). `.github/workflows/release.yml` checks the tag against the
version, builds and smoke-tests the Linux x86_64/aarch64 and Apple Silicon
binaries, publishes the GitHub Release and installs it with `install.sh` on
each platform; started by hand it only builds and checks. `install.sh` is
tested by `tests/install.rs` against a fake release.

Updates are `src/update.rs`: the lookup of the latest release (`<base>/latest`
redirects to the tag page; the redirect is read, not followed), the check
remembered in `update-check.json` in the data directory, the notice that
`hook session-start` appends, and `recollect update`, which pipes the
`install.sh` embedded in the binary to `sh`. Old binaries install new
releases with the script they were built with, so the release files keep
their names and layout (`recollect-<target>.tar.gz`, `SHA256SUMS`,
`download/<tag>/`). A development build looks up and installs releases only
from `RECOLLECT_DOWNLOAD_BASE`; the tests point it at
`tests/common/release.rs`'s server and never reach GitHub.

Sync between machines is `src/sync/`: `identity` (the machine's key,
`identity.key` in the data directory), `protocol` (the JSON messages and
their framing), `exchange` (manifest, diff, validation, batching),
`transport` (TLS with pinned key fingerprints), `round` (one round),
`pairing` (invites) and `daemon` (`recollect serve`); peers and invites are
tables in `memories.db` (`src/db/peers.rs`), the rows a round moves go
through `src/db/sync.rs`. Embeddings and local ids never travel. Whenever a
change alters what goes over the wire (a message, a record field, a value a
field may take, such as a new memory type), bump `PROTOCOL_VERSION` in
`src/sync/protocol.rs` and update the fixtures in its tests, which pin the
exact JSON; releases and schema migrations that leave the wire alone do not
touch it. `cargo test --test sync` runs the end-to-end tests: the real
binary on several data directories, talking over local sockets.

The data directory also holds `memories.db.lock`. Every process that opens the
database (the CLI, a sync daemon) must take a blocking `flock` on it around the
switch to WAL mode, because SQLite does not invoke the busy handler for that
switch.

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
- **Resources** (`lib/recollect/resources/`): MCP resources - per-project markdown context (`Projects`) and the single-memory template (`Memory`), rendered by `MemoryMarkdown`

### Design Decisions

- **HTTP-only transport**: No stdio; single Puma server simplifies SQLite concurrency
- **MCP via handle_json**: MCP protocol exposed at `/mcp` endpoint
- **Project isolation**: Separate database per project, plus global database
- **Vector search**: Optional hybrid FTS5 + vector similarity search via sqlite-vec extension
- **Three distribution channels**: the gem ships the server and CLI (`exe/`), the Claude Code
  plugin ships the agent-facing parts (`skills/`, `hooks/`, all working through
  the Rust `recollect` binary), catalogued by `.claude-plugin/marketplace.json` so this
  repository is its own marketplace, and GitHub Releases ship the Rust `recollect`
  binary (`install.sh`)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data storage directory |
| `RECOLLECT_HOST` | `127.0.0.1` | Server bind address |
| `RECOLLECT_PORT` | `7326` | Server port |
| `RECOLLECT_URL` | `http://localhost:7326` | CLI base URL |
| `RECOLLECT_ENABLE_VECTORS` | `false` | Enable vector search |
| `RECOLLECT_MAX_VECTOR_DISTANCE` | `1.0` | Max cosine distance (0-2) for vector results |
| `RECOLLECT_SQLITE_VEC_PATH` | (auto-detect) | Path to the sqlite-vec extension, checked before built-in locations |
| `RECOLLECT_PYTHON` | `.venv/bin/python3`, else `python3` | Python interpreter running the embedding model |
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

For Rust changes, also run:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

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

