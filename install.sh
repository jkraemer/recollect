#!/bin/sh
# Installs or upgrades recollect from a GitHub release:
#
#   curl -fsSL https://raw.githubusercontent.com/jkraemer/recollect/master/install.sh | sh
#
# RECOLLECT_VERSION        a release tag such as v0.1.0; default: the latest release
# RECOLLECT_INSTALL_DIR    where recollect goes; default: $HOME/.local/bin
# RECOLLECT_DOWNLOAD_BASE  where releases live; default: the project's GitHub releases
set -eu

base="${RECOLLECT_DOWNLOAD_BASE:-https://github.com/jkraemer/recollect/releases}"
install_dir="${RECOLLECT_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
  echo "error: $*" >&2
  exit 1
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os $arch" in
  "Linux x86_64") target=x86_64-unknown-linux-gnu ;;
  "Linux aarch64" | "Linux arm64") target=aarch64-unknown-linux-gnu ;;
  "Darwin arm64") target=aarch64-apple-darwin ;;
  *) fail "no prebuilt recollect for $os $arch; build it from source: cargo install --git https://github.com/jkraemer/recollect --locked" ;;
esac

command -v curl > /dev/null 2>&1 || fail "curl is required to download recollect"

if [ -n "${RECOLLECT_VERSION:-}" ]; then
  url="$base/download/$RECOLLECT_VERSION"
else
  url="$base/latest/download"
fi
archive="recollect-$target.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for file in "$archive" SHA256SUMS; do
  curl -fsSL -o "$tmp/$file" "$url/$file" || fail "could not download $url/$file"
done

# Checks the files listed in the checksum file $1, relative to the current directory.
verify_sha256() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum -c "$1"
  else
    shasum -a 256 -c "$1"
  fi
}

awk -v archive="$archive" '$2 == archive' "$tmp/SHA256SUMS" > "$tmp/expected"
[ -s "$tmp/expected" ] || fail "SHA256SUMS lists no $archive"
(cd "$tmp" && verify_sha256 expected > /dev/null 2>&1) || fail "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp"
version="$("$tmp/recollect" --version 2>&1)" ||
  fail "the downloaded recollect does not run on this machine: $version"

mkdir -p "$install_dir"
cp "$tmp/recollect" "$install_dir/.recollect.new"
chmod 755 "$install_dir/.recollect.new"
mv -f "$install_dir/.recollect.new" "$install_dir/recollect"

echo "installed $version to $install_dir/recollect"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "warning: $install_dir is not on PATH" >&2 ;;
esac
found="$(command -v recollect 2> /dev/null || true)"
if [ -n "$found" ] && [ "$found" != "$install_dir/recollect" ]; then
  echo "warning: $found comes first on PATH and shadows $install_dir/recollect" >&2
fi
echo "The embedding model (about 30 MB) downloads on first use."
