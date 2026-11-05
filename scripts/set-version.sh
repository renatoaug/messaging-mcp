#!/bin/sh
# Called by semantic-release with the next version: writes it to Cargo.toml and Cargo.lock.
# Edits both files directly: the release runner has no crate index, so cargo cannot rewrite the lock.
set -e
version="$1"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "usage: scripts/set-version.sh X.Y.Z" >&2; exit 1 ;;
esac
# The [package] version is the first `version =` line in Cargo.toml.
awk -v version="$version" '
  !done && /^version = / { print "version = \"" version "\""; done = 1; next }
  { print }
' Cargo.toml > Cargo.toml.next
mv Cargo.toml.next Cargo.toml
# In Cargo.lock, the version line right after this package's name.
awk -v version="$version" '
  /^name = "messaging-mcp"$/ { print; mine = 1; next }
  mine && /^version = / { print "version = \"" version "\""; mine = 0; next }
  { print }
' Cargo.lock > Cargo.lock.next
mv Cargo.lock.next Cargo.lock
