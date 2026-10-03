#!/usr/bin/env bash
# Runs the plugin's eval suite with `claude plugin eval`; extra arguments
# (--model, --case, --runs, --ablation, ...) are passed through.
#
# `claude plugin eval` refuses a plugin directory that contains hard links,
# and cargo's target/ is full of them, so the plugin's files are staged into
# a temporary directory first, together with the embedding model that the
# cases copy into each run's home (the run's sandbox cannot download it).
# The `recollect` binary on PATH is the one under test.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
if [ ! -d "$repo/.model-cache" ]; then
  echo "error: $repo/.model-cache is missing; run cargo test once to download the model" >&2
  exit 1
fi

stage="$(mktemp -d "${TMPDIR:-/tmp}/recollect-evals.XXXXXX")"
cp -r "$repo/.claude-plugin" "$repo/skills" "$repo/hooks" "$repo/evals" "$stage/"
cp -rL "$repo/.model-cache" "$stage/.eval-model"

echo "staged plugin: $stage"
claude plugin eval "$stage" "$@" --scaffold --trust-plugin --no-publish \
  --allow-tools "Bash(recollect *)"
