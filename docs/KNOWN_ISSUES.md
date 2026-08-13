# Known Issues

Open items found while building the CI pipeline. Most of the original list was
fixed on the `fix/known-issues` branch (one commit per item, each citing its
item number); what remains below is deliberately unfixed, with the reason and
what fixing it would take. The **Fixed** section at the end records what was
done, so the history stays legible.

## Current CI state

Two GitHub Actions workflows on `master`, both green:

- `.github/workflows/ci.yml` — on push to `master` and PRs against it. A `lint`
  job (`rake rubocop`) and a `test` job (`rake test`) on Ruby 3.4.
- `.github/workflows/nightly.yml` — `schedule` at `27 18 * * *` UTC plus
  `workflow_dispatch`. Installs CPU-only torch, `sentence-transformers` and
  `sqlite-vec`, runs `bin/verify-vector-stack`, then `rake coverage`.

The nightly is the only thing that executes vector code and the only thing that
enforces `minimum_coverage line: 80, branch: 70` (`test/test_helper.rb`).

The nightly's residual skips are correct: `memories_service_test.rb`'s
`skip_if_vectors_enabled` guards two tests that only make sense with vectors
*off*. They run in the fast job and skip in the nightly. The two jobs are
complementary, not superset/subset — do not "fix" the skip count to zero.

`bin/verify-vector-stack` exists so a misconfigured nightly fails loudly instead
of reporting success having tested nothing: the vector tests *skip* rather than
fail when the stack is missing. It opens a real database with
`load_vectors: true`, asserts post-load `vectors_enabled?`, and does a live
embedding round-trip. The post-load gating in `database_vector_test.rb`
(fixed item 6) is only safe while this guard stays that strict.

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
  extension — plus one truthful failure:
  `test_load_vector_extension_succeeds` is deliberately gated on the
  `File.exist?` prediction, so a present-but-unloadable extension fails it.
  Everything else in `database_vector_test.rb` gates on the database's real
  post-load state and skips. Plain `rake test` must stay pristine.

- That simulation proves nothing about actual vector *behavior*, since the fake
  extension never loads. For anything touching real vector code, the nightly is
  the only proof: `gh workflow run nightly.yml --repo jkraemer/recollect`.

`TESTOPTS="--seed N"` breaks Rake's argument parsing here; use `--seed=N`.

Tests must control their own environment rather than inheriting it.
`Recollect::EnvHelpers#with_env` (`test/test_helper.rb`, included by
`Recollect::TestCase` and includable anywhere) snapshots and restores; a `nil`
value means "unset for the duration". Use singleton-method stubbing instead
where the value is memoized at `Config.new` time (`enable_vectors`) or the
object was built in `setup`.

---

## Open items

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

### The Ruby matrix is `["3.4"]` only

3.5 was removed because `setup-ruby` resolves `"3.5"` to `3.5.0-preview1` and
`sqlite3` caps itself at `< 3.5.dev` — which excludes 3.5.0 final too. Re-add
once sqlite3 ships 3.5 support; `test/ci_workflow_test.rb` enforces that the
matrix covers the gemspec's `required_ruby_version` floor.

### Extension check in sqlite instead of path probing

`docs/TODO.md` asks whether the extension check could be done *in sqlite*
instead of probing a list of install locations. Same theme as the fixed item 6
— predicted versus actual availability. `Config#vec_extension_path` still
predicts; only the tests and the nightly guard verify reality.

## Fixed

Each item below was fixed on `fix/known-issues`, one commit per item (the
commit message carries the full reasoning; `git log --grep` the item number).

1. **`delete_memory` could delete the wrong memory across projects** —
   `project` is now a required (nullable) tool argument, and
   `Schemas::MEMORY` lists `project` as required so every result carries the
   value a caller must pass back.
2. **A project named `global` shadowed the global database** — the name is
   reserved in `sanitize_project_name`, covering every path through
   `get_database`; `ArgumentError` maps to HTTP 400 via an error handler.
3. **Recency applied twice to the FTS arm of `hybrid_search`** — the FTS arm
   now uses the recency-free `fts_search`; recency applies once, at merge
   time, to both arms.
5. **`determine_vector_unavailable_reason` duplicated
   `vector_status_message`** — the cascade lives once in
   `Config#vector_unavailable_reason`; message and endpoint derive from it.
6. **`database_vector_test.rb` gated on a prediction** — vector-behavior tests
   now gate on post-load `vectors_enabled?` via `open_vector_database_or_skip`;
   only `test_load_vector_extension_succeeds` keeps the prediction gate, on
   purpose (on-disk-but-unloadable must fail there, mirroring
   `bin/verify-vector-stack`).
7. **`with_env` unreachable outside `Recollect::TestCase`** — extracted to
   `Recollect::EnvHelpers`; all known raw `ENV` sites converted, including two
   whose cleanup deleted ambient values instead of restoring them.
8. **`test_default_data_dir` created the real `~/.recollect`** — `HOME` points
   into the test sandbox for the test's duration; `test_data_dir_from_env` no
   longer writes `/tmp/recollect_test` either.
9. **Two tests didn't test what their names claimed** — the hybrid recency
   test lost its pointless vector skip; the enqueue-noop test constructs its
   own vectors-off manager instead of trusting the ambient config.
10. **Ranker `project` precondition unenforced** — `rrf_merge` reads `project`
    with `fetch`, so a producer omitting it raises `KeyError` instead of
    silently colliding; fixtures and two rejection tests pin the contract.
11. **Stderr noise in `embedding_worker_test.rb`** — the enqueue test no
    longer starts a live consumer thread, which was failing batches
    asynchronously on machines without the vector stack.
- **CI hardening** — PR-only `cancel-in-progress`, nightly `concurrency`
  group, `timeout-minutes` on ci.yml jobs, per-run HF cache key with prefix
  `restore-keys`, `sqlite-vec` homed in `requirements.txt` with a version
  floor. Each invariant is pinned by a test in `ci_workflow_test.rb`.
