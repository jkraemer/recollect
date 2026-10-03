#!/usr/bin/env bash
# Runs the plugin's eval suite with `claude plugin eval`; extra arguments
# (--model, --case, --runs, --ablation, ...) are passed through.
#
# `claude plugin eval` refuses a plugin directory that contains hard links,
# and cargo's target/ is full of them, so the plugin's files are staged into
# a temporary directory first, together with the embedding model that the
# cases copy into each run's home (the run's sandbox cannot download it).
# Results are copied back to evals/results/ and the staging directory is
# removed, whatever the outcome. The `recollect` first on PATH is under test.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
if [ ! -d "$repo/.model-cache" ]; then
  echo "error: $repo/.model-cache is missing; run cargo test once to download the model" >&2
  exit 1
fi
echo "recollect under test: $(command -v recollect) ($(recollect --version))"

stage="$(mktemp -d "${TMPDIR:-/tmp}/recollect-evals.XXXXXX")"
# claude plugin eval exits 1 whenever a case scores below the threshold, so
# the results are saved from a trap rather than after the command.
save_results_and_clean_up() {
  if [ -d "$stage/evals/results" ]; then
    mkdir -p "$repo/evals/results"
    cp -r "$stage/evals/results/." "$repo/evals/results/"
    echo "results: $repo/evals/results/"
  fi
  rm -rf "$stage"
}
trap save_results_and_clean_up EXIT

cp -r "$repo/.claude-plugin" "$repo/skills" "$repo/hooks" "$repo/evals" "$stage/"
rm -rf "$stage/evals/results"
cp -rL "$repo/.model-cache" "$stage/.eval-model"

# Write and Edit let a run save to Claude Code's auto memory, which the
# storage cases compare against; each run's home is a throwaway directory.
claude plugin eval "$stage" "$@" --scaffold --trust-plugin --no-publish \
  --allow-tools Write Edit "Bash(recollect *)"
