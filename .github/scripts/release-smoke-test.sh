#!/bin/sh
# Checks a freshly built release binary on the machine that built it: its
# version, and storing and finding a memory, which loads the embedding model
# and with it the ONNX runtime linked into the binary.
#
#   sh .github/scripts/release-smoke-test.sh <binary> <expected version>
set -eu

binary="$1"
expected="$2"
data="$(mktemp -d)"
logs="$(mktemp -d)"
trap 'rm -rf "$data" "$logs"' EXIT
export RECOLLECT_DATA_DIR="$data"

fail() {
  echo "error: $*" >&2
  exit 1
}

version="$("$binary" --version)"
[ "$version" = "recollect $expected" ] ||
  fail "$binary reports '$version', expected 'recollect $expected'"

"$binary" store "release smoke test" 2> "$logs/store.err" || { cat "$logs/store.err" >&2; fail "store failed"; }
if grep -v '^downloading embedding model ' "$logs/store.err" > "$logs/unexpected"; then
  cat "$logs/unexpected" >&2
  fail "store wrote unexpected output to stderr"
fi

"$binary" search "smoke test" --json | grep -q '"content":"release smoke test"' ||
  fail "search did not find the stored memory"
"$binary" status --json | grep -q '"pending_embeddings":0' ||
  fail "the memory was stored without vectors"

echo "smoke test passed: $version"
