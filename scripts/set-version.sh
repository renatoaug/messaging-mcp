#!/bin/sh
# Called by semantic-release with the next version: writes it to Cargo.toml and Cargo.lock.
set -e
version="$1"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "usage: scripts/set-version.sh X.Y.Z" >&2; exit 1 ;;
esac
# Only the [package] version line, which is the first `version =` in the file.
awk -v version="$version" '!done && /^version = / { print "version = \"" version "\""; done = 1; next } { print }' \
  Cargo.toml > Cargo.toml.next
mv Cargo.toml.next Cargo.toml
cargo update --workspace --offline
