#!/bin/sh
# Installs or upgrades recollect from a GitHub release:
#
#   curl -fsSL https://raw.githubusercontent.com/jkraemer/recollect/master/install.sh | sh
#
# RECOLLECT_VERSION        a release tag such as v0.1.0; default: the latest release
# RECOLLECT_INSTALL_DIR    where recollect goes; default: $HOME/.local/bin
# RECOLLECT_DOWNLOAD_BASE  where releases live; default: the project's GitHub releases
# RECOLLECT_CPUINFO        the file listing the CPU's features; default: /proc/cpuinfo
set -eu

base="${RECOLLECT_DOWNLOAD_BASE:-https://github.com/jkraemer/recollect/releases}"
install_dir="${RECOLLECT_INSTALL_DIR:-$HOME/.local/bin}"
# A trailing slash would make the PATH checks below compare "dir/" with "dir".
install_dir="${install_dir%/}"
cpuinfo="${RECOLLECT_CPUINFO:-/proc/cpuinfo}"

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
  *) fail "no prebuilt recollect for $os $arch" ;;
esac

command -v curl > /dev/null 2>&1 || fail "curl is required to download recollect"

# The x86_64 build needs AVX2: the ONNX Runtime linked into it is compiled
# for it, and without it the binary dies with "Illegal instruction" before it
# can say why. Virtual machines often hide AVX2 from the guest. A machine that
# does not list its CPU's features is not refused: awk succeeds only when the
# features are listed and AVX2 is not among them.
if [ "$target" = x86_64-unknown-linux-gnu ] && [ -r "$cpuinfo" ] &&
  awk '/^flags/ { listed = 1; for (i = 1; i <= NF; i++) if ($i == "avx2") found = 1 }
       END { if (listed && !found) exit 0; exit 1 }' "$cpuinfo"; then
  fail "this CPU has no AVX2, which the prebuilt recollect needs (in a virtual machine, choose a CPU type that passes it through, such as \"host\")"
fi

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
  fail "the downloaded recollect does not run on this machine${version:+: $version}"

mkdir -p "$install_dir"
# An upgrade has its embedding model already.
first_install=true
[ -e "$install_dir/recollect" ] && first_install=false
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
if "$first_install"; then
  echo "The embedding model (about 65 MB) downloads on first use."
fi
