# Known Issues

Open items found while building the CI pipeline. Everything here is deliberately
unfixed; each entry says why, and what fixing it would take.

## Current CI state

Two GitHub Actions workflows on `master`, both green:

- `.github/workflows/ci.yml` — on push to `master` and PRs against it. A `lint`
  job (`rake rubocop`) and a `test` job (`rake test`) on Ruby 3.4.
- `.github/workflows/nightly.yml` — `schedule` at `27 18 * * *` UTC plus
  `workflow_dispatch`. Installs CPU-only torch, `sentence-transformers` and
  `sqlite-vec`, runs `bin/verify-vector-stack`, then `rake coverage`.

The nightly is the only thing that executes vector code and the only thing that
enforces `minimum_coverage line: 80, branch: 70` (`test/test_helper.rb`). Last
run: 559 tests, 2 skips, 94.14% line / 81.34% branch, ~16m30s against a
30-minute timeout.

The 2 skips are correct: `memories_service_test.rb`'s `skip_if_vectors_enabled`
guards two tests that only make sense with vectors *off*. They run in the fast
job and skip in the nightly. The two jobs are complementary, not
superset/subset — do not "fix" the skip count to zero.

`bin/verify-vector-stack` exists so a misconfigured nightly fails loudly instead
of reporting success having tested nothing: the vector tests *skip* rather than
fail when the stack is missing. It opens a real database with
`load_vectors: true`, asserts post-load `vectors_enabled?`, and does a live
embedding round-trip. Several items below depend on it staying that strict.

## Working on these

This machine has no `.venv` and no real sqlite-vec extension, so vector code
cannot execute locally. Two consequences:

- The CI environment can be *simulated* for environment-assumption bugs, because
  `Config#vectors_available?` only does `File.exist?` on the extension path:

  ```bash
  RECOLLECT_ENABLE_VECTORS=true RECOLLECT_PYTHON=$(which python3) \
    RECOLLECT_SQLITE_VEC_PATH=/etc/hostname bundle exec rake test
  ```

  Expect `Failed to load sqlite-vec` on stderr — the fake path is not a real
  extension. Plain `rake test` must stay pristine.

- That simulation proves nothing about actual vector *behavior*, since the fake
  extension never loads. For anything touching real vector code, the nightly is
  the only proof: `gh workflow run nightly.yml --repo jkraemer/recollect`.

`TESTOPTS="--seed N"` breaks Rake's argument parsing here; use `--seed=N`.

Tests must control their own environment rather than inheriting it.
`Recollect::TestCase#with_env` (`test/test_helper.rb`) snapshots and restores;
a `nil` value means "unset for the duration". Use singleton-method stubbing
instead where the value is memoized at `Config.new` time (`enable_vectors`) or
the object was built in `setup`.

---

## Production bugs

### 1. `delete_memory` can delete the wrong memory across projects

`lib/recollect/tools/delete_memory.rb:33`, `lib/recollect/memories_service.rb:69`.

`project` is optional and defaults to `nil`, which resolves to the global
database. But `search_memory` with no project returns results from every
project, and `Schemas::MEMORY` (`lib/recollect/schemas.rb:21`) does not list
`project` as required — so a search result need not carry it, and a model has no
signal that it must be passed back. Reading `id: 1` from a project-scoped result
and deleting without `project` deletes *global* memory 1.

Silent data loss on the tool's own default path. The web UI already threads
`data-project` through correctly, which suggests the API contract is understood
to need it.

Fix direction: make the memory's project part of its identity at the tool
boundary — either require `project` on delete, or add it to `Schemas::MEMORY`'s
required fields so results always carry it. This is an API contract decision.

### 2. A project named `global` shadows the global database

`MemoriesService#create` rejects the name, but the read paths do not.
`DatabaseManager#get_database("global")` (`lib/recollect/database_manager.rb:18`)
creates `projects/global.db`; `list_db_names` (`:182`) then returns `"global"`
twice, and `project_for_db_name` (`:178`) maps both to `nil`.

Fix direction: reserve the name in `get_database` / `sanitize_project_name`
(`:259`) rather than only at creation.

### 3. Recency is applied twice to the FTS arm of `hybrid_search`

`search_all` already recency-ranks at `lib/recollect/database_manager.rb:80`,
and its output feeds `HybridSearchRanker.merge` with a second `recency_ranker`
at `:157`. The vector arm gets it once.

The sign handling in `apply_recency_ranking` (`:338`) is correct, so this is a
weighting asymmetry between the two arms, not an inverted sort.

### 4. `DatabaseManager.new` spawns a thread as a construction side effect

`lib/recollect/database_manager.rb:15` — `start_embedding_worker if
@config.vectors_available?`. `EmbeddingWorker#start` immediately calls
`recover_missing_embeddings`, which calls `get_database(nil)` and caches a
vector-enabled database.

A constructor that starts a thread and opens databases makes tests race: any
stub applied after construction is already too late. One test was fixed by
constructing a vectors-off config *first* rather than stubbing afterwards; the
underlying design is unchanged.

Architectural — deliberately left for a decision rather than patched.

### 5. `determine_vector_unavailable_reason` duplicates `vector_status_message`

`lib/recollect/http_server.rb:234` and `lib/recollect/config.rb:105` compute the
same three-branch cascade from the same predicates in the same order. They can
only diverge if someone edits one.

No live defect. The endpoint wants the bare reason and the config wants a wrapped
sentence, so the clean fix is exposing the reason from `Config` and having the
endpoint use it.

## Test suite

### 6. `database_vector_test.rb` gates on a prediction, not reality — now unblocked

`test/recollect/database_vector_test.rb:243` skips on
`Recollect.config.vec_extension_path`, which is a `File.exist?` check, rather
than the database's real post-load `vectors_enabled?`. 12 tests are affected.

This was previously *unsafe to fix*: while the nightly guard only predicted
availability, this test was the only thing that turned a present-but-unloadable
`vec0.so` into a red nightly. Re-pointing it would have created exactly the
hollow green the guard exists to prevent.

`bin/verify-vector-stack` now performs that check, so the constraint is
satisfied and this can be fixed. Verify against a real nightly run, not the
local simulation.

Related: `docs/TODO.md` asks whether the extension check could be done *in
sqlite* instead of probing a list of install locations. Same theme — predicted
versus actual availability — and worth resolving together.

### 7. `with_env` is unreachable from three test files

`test/recollect/sync/endpoint_test.rb:5` and
`test/integration/sync_two_peer_test.rb:5` subclass `Minitest::Test` directly,
and `test/integration/two_peer_helper.rb` is a mixin — none can reach a helper
defined on `Recollect::TestCase`.

This is the structural reason three separate sweeps each left sites behind.
Extracting `with_env` into a module both hierarchies can include closes the
class of defect instead of chasing instances.

Outstanding raw `ENV` sites: `RECOLLECT_SYNC_DISABLE`
(`http_server_test.rb`, `integration/sync_two_peer_test.rb`),
`RECOLLECT_PUBLIC_URL` (`sync/endpoint_test.rb`), and `RECOLLECT_DATA_DIR`
(`test/integration/two_peer_helper.rb:63-66`) — the last with **no `ensure`**,
so a raise leaves it pointing at a temp directory for the rest of the process.

All three vector variables (`RECOLLECT_ENABLE_VECTORS`, `RECOLLECT_PYTHON`,
`RECOLLECT_SQLITE_VEC_PATH`) are fully converted; none of the above is set by
CI, so none is a live failure today.

### 8. `test_default_data_dir` creates the real `~/.recollect`

`test/recollect/config_test.rb`, `test_default_data_dir`. It deletes
`RECOLLECT_DATA_DIR` and calls `Config.new`, whose `ensure_directories!`
`mkpath`s `~/.recollect` and `~/.recollect/projects` — on the developer's
machine and on the runner.

### 9. Two tests no longer test what their names claim

- `test/recollect/database_manager_test.rb:484`
  `test_hybrid_search_applies_recency_when_enabled` skips unless vectors are
  available, but with vectors off `hybrid_search` returns `search_all`, which
  applies recency itself at `:80`. Its sibling `test_search_all_applies_recency_when_enabled`
  asserts the same two things with no guard. The skip serves no purpose.
- `test/recollect/database_manager_test.rb:446`
  `test_enqueue_embedding_noop_when_vectors_disabled` assumes `@embedding_worker`
  is nil. Under the nightly it is not, so the test enqueues a live job instead of
  exercising the `&.` path. It passes either way, which is how it went unnoticed.

### 10. The ranker's `project` precondition is unenforced

`HybridSearchRanker.rrf_merge` keys its score map by `[project, id]` because ids
are per-database `AUTOINCREMENT` values and collide across projects. Every
current producer stamps `m["project"]`, but
`test/recollect/hybrid_search_ranker_test.rb:88` (`test_merge_weighting`) passes
hashes with no `project` key at all and passes — so the suite documents
project-less input as acceptable.

Three of five producers set `project` in a separate `.each` *after* the query,
which is the shape that gets forgotten. A future producer that omits it silently
reintroduces the collision for its own results.

Fix direction: state the precondition on `rrf_merge`, or assert in
`database_manager` tests that every element reaching the ranker carries
`project`.

### 11. Stderr noise in `embedding_worker_test.rb`

`Batch failed: ... broken pipe` appears during the run. Pre-existing, and
against the project rule that test output must be pristine — expected errors
should be captured and asserted rather than printed.

## CI hardening

None of these is a live failure; all are cheap.

- `ci.yml` sets `cancel-in-progress: true` for a group keyed on `github.ref`,
  which applies to master pushes as well as PRs. Two quick pushes to master
  leave the first commit with no CI result. Conventional form:
  `cancel-in-progress: ${{ github.event_name == 'pull_request' }}`.
- `nightly.yml` has no `concurrency` group, so a `workflow_dispatch` can race the
  cron over the same cache key.
- `ci.yml` has no `timeout-minutes`, so a hung run burns the 360-minute default.
- The HF model cache uses a fixed `key` with no `restore-keys`. Once saved,
  every run is an exact hit and the save step never runs again — a partial entry
  would be permanently sticky, recoverable only by deleting the cache or bumping
  the key string.
- `sqlite-vec` is unpinned and declared only in `nightly.yml`, not in
  `requirements.txt`. No false-green path exists (a breaking release fails the
  guard or the install), but `requirements.txt` would give it one home.
- The Ruby matrix is `["3.4"]` only. 3.5 was removed because `setup-ruby`
  resolves `"3.5"` to `3.5.0-preview1` and `sqlite3` caps itself at
  `< 3.5.dev` — which excludes 3.5.0 final too. Re-add once sqlite3 ships 3.5
  support; `test/ci_workflow_test.rb` enforces that the matrix covers the
  gemspec's `required_ruby_version` floor.

## Suggested order

1. **Item 1** — user-facing silent data loss on a default path.
2. **Item 6** — now unblocked, and restores real coverage of the vector code.
3. **Item 7** — closes the defect class rather than another instance of it.

Items 2, 3, 9, 10 are correctness issues without a known trigger in normal use.
Items 4 and 5 are design decisions. The CI hardening items are opportunistic.
