# AGENTS.md

Guidance for AI coding agents working in this repository. `CLAUDE.md` and `GEMINI.md`
are symlinks to this file.

When storing or recalling memories, refer to this project as "recollect".

## Project Overview

**Recollect** is a local-first command-line tool that gives coding agents a
persistent memory: one Rust binary that keeps memories in a single SQLite
database with FTS5 full-text search and vector search, and can sync them
between machines. The crate is at the repository root (`Cargo.toml`, `src/`,
`tests/`). The repository is also the Claude Code plugin (`skills/`, `hooks/`)
and its own marketplace (`.claude-plugin/marketplace.json`). There is no
server and no MCP: agents run the CLI.

## Commands

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

Run `cargo test` without `RECOLLECT_DATA_DIR` set: the test of that default
fails otherwise, by design. `target/test-data` may be deleted at any time, and
has to be after a checkout with a newer schema has used it.

`recollect migrate-from-ruby` (`src/migrate.rs`) copies the memories of the
retired Ruby server's data directory (`global.db`, `projects/*.db`) into the
database. It stays for installations that are not migrated yet; its tests
build Ruby-shaped data directories with `tests/common/ruby.rs`. The Ruby
server itself is in git up to the tag `v0.3.0`.

The Claude Code plugin's hooks (`hooks/hooks.json`) run `recollect hook
session-start` and `recollect hook post-compact` with Claude Code's hook
input on stdin; that corner of the CLI is `src/hook.rs`, with project
detection (a `.recollect-project` file, else the git repository's directory
name) in `src/detect.rs`.

`evals/` holds `claude plugin eval` cases for the skill: whether an agent
searches recollect before answering, stores decisions in the right project,
keeps working preferences in auto memory instead, and passes the update notice on to the user without running the update. `evals/run.sh` stages the plugin (cargo's hard links in `target/`
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
through `src/db/sync.rs`. The daemon compares the device and inode of its
executable before every timer pass and every answered connection and `exec`s
the new file after an update (`Restart` in `daemon.rs`): it opens the database
per round, so without that an old daemon would fail every round once a new CLI
has migrated the schema. Embeddings and local ids never travel. Whenever a
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
main.rs      the clap CLI: parses a command, prints its result
service.rs   one method per command, combining storage, embeddings and search
db/          SQLite: schema migrations, memories, chunks, queries, peers
embed/       the chunker and the in-process model behind the Embedder trait
search/      the FTS5 query, reciprocal rank fusion, recency decay
sync/        sync between machines (see above)
hook.rs      the Claude Code hooks, with detect.rs
update.rs    the update check and recollect update
migrate.rs   reading a Ruby installation's data
output.rs    text rendering; --json goes through serde
```

### Design Decisions

- **CLI only**: no server, no MCP; the plugin's hooks and skills run the binary
- **One database**: `memories.db` holds every project in a `project` column
  (NULL means global); the CLI and the sync daemon open the same file
- **Immutable memories**: a memory is stored and at most tombstoned, never
  edited, which is what lets sync take the union of two machines
- **Embeddings in-process**: `bge-small-en-v1.5`, quantized, run by fastembed;
  a memory whose embedding fails is stored without vectors and embedded later
- **Hybrid search**: FTS5 and sqlite-vec candidates, merged by reciprocal rank
  fusion
- **Two distribution channels**: GitHub Releases ship the binary
  (`install.sh`), and the Claude Code plugin ships the agent-facing parts
  (`skills/`, `hooks/`), catalogued by `.claude-plugin/marketplace.json` so
  this repository is its own marketplace

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `RECOLLECT_DATA_DIR` | `~/.recollect` | Data directory; a development build has no default |
| `RECOLLECT_MODEL_DIR` | `<data dir>/models` | Embedding model cache |
| `RECOLLECT_DOWNLOAD_BASE` | GitHub Releases | Where `recollect update` and `install.sh` find releases; the only source a development build uses |
| `RECOLLECT_VERSION` | latest | Release tag `install.sh` installs |
| `RECOLLECT_INSTALL_DIR` | `~/.local/bin` | Where `install.sh` puts the binary |

Search, recency ranking, sync and the update check are tuned in
`config.toml` in the data directory (`src/config.rs`).

## Before Committing

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Run test coverage and ensure it hasn't degraded:

```bash
cargo llvm-cov --fail-under-lines 80
```

Degrading test coverage is strongly discouraged. If coverage drops, add tests for uncovered code before committing.

## Testing

Unit tests sit in their modules under `src/`. The integration tests in
`tests/` run the library and the real binary on temporary data directories;
`tests/common/` holds what they share (the embedding model loaded once, a
fake release site, Ruby-shaped data).
