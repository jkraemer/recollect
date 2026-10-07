#!/bin/sh
# Checks a freshly built release binary on the machine that built it: its
# version, storing and finding a memory, which loads the embedding model and
# with it the ONNX runtime linked into the binary, and its default data
# directory.
#
#   sh .github/scripts/release-smoke-test.sh <binary> <expected version>
set -eu

binary="$1"
expected="$2"
data="$(mktemp -d)"
logs="$(mktemp -d)"
home="$(mktemp -d)"
trap 'rm -rf "$data" "$logs" "$home"' EXIT
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

# Without RECOLLECT_DATA_DIR a release build uses ~/.recollect. Every installed
# recollect starts that way (the plugin's hooks, the sync daemon), and nothing
# else checks it: the test suite runs development builds, which refuse to
# start without the variable.
status="$(unset RECOLLECT_DATA_DIR && HOME="$home" "$binary" status --json)" ||
  fail "status failed without RECOLLECT_DATA_DIR"
printf '%s\n' "$status" | grep -qF "\"data_dir\":\"$home/.recollect\"" ||
  fail "without RECOLLECT_DATA_DIR the data directory is not ~/.recollect: $status"

echo "smoke test passed: $version"
