#!/bin/sh
# Installs messaging-mcp and its WhatsApp bridge from a GitHub release, checks the download's
# SHA-256, and registers the server in Claude Code and Codex when they are installed.
#
#   curl -fsSL https://raw.githubusercontent.com/renatoaug/messaging-mcp/main/install.sh | sh
#
# Settings, all optional:
#   MESSAGING_MCP_VERSION=v0.2.1           a release tag (default: the latest)
#   MESSAGING_MCP_INSTALL_DIR=~/.local/bin  where both binaries go
#   MESSAGING_MCP_REGISTER=0               do not register in Claude Code or Codex
#   MESSAGING_MCP_APPROVAL=client-prompt   or "terminal": skip the question about approvals
set -eu

REPO="renatoaug/messaging-mcp"
DIR="${MESSAGING_MCP_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() { printf 'messaging-mcp install: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux) os="unknown-linux-gnu" ;;
  *) fail "only macOS and Linux are supported." ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch="aarch64" ;;
  x86_64 | amd64) arch="x86_64" ;;
  *) fail "unsupported processor: $(uname -m)." ;;
esac
target="$arch-$os"

command -v curl >/dev/null 2>&1 || fail "curl is required."
command -v tar >/dev/null 2>&1 || fail "tar is required."

version="${MESSAGING_MCP_VERSION:-}"
if [ -z "$version" ]; then
  # The latest release page redirects to /releases/tag/<version>.
  latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")
  version="${latest##*/}"
fi
case "$version" in
  v[0-9]*) ;;
  *) fail "could not find the latest release." ;;
esac

name="messaging-mcp-$version-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$version/$name"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

say "Downloading messaging-mcp $version for $target..."
if ! curl -fsSL "$url" -o "$work/$name"; then
  # A release is published a few minutes before its binaries are attached.
  fail "$name is not available yet. If $version was just released, try again in a few minutes, or pick an earlier one: MESSAGING_MCP_VERSION=<tag>."
fi
curl -fsSL "$url.sha256" -o "$work/$name.sha256" || fail "checksum download failed."

expected=$(cut -d ' ' -f 1 <"$work/$name.sha256")
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$work/$name" | cut -d ' ' -f 1)
else
  actual=$(shasum -a 256 "$work/$name" | cut -d ' ' -f 1)
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "checksum mismatch; nothing was installed."

tar -xzf "$work/$name" -C "$work"
mkdir -p "$DIR"
for binary in messaging-mcp messaging-mcp-whatsapp-bridge; do
  [ -f "$work/$binary" ] || fail "$binary is missing from the release archive."
  install -m 755 "$work/$binary" "$DIR/$binary"
done
say "Installed messaging-mcp and messaging-mcp-whatsapp-bridge in $DIR."

# The config is created on first use; this also checks the binary runs here.
"$DIR/messaging-mcp" doctor >/dev/null || fail "messaging-mcp does not run on this system."

if [ "${MESSAGING_MCP_REGISTER:-1}" != "0" ]; then
  if command -v claude >/dev/null 2>&1; then
    if claude mcp get messaging-mcp >/dev/null 2>&1; then
      say "Claude Code: messaging-mcp is already registered."
    else
      claude mcp add --scope user messaging-mcp -- "$DIR/messaging-mcp" mcp >/dev/null &&
        say "Claude Code: registered messaging-mcp for all your projects."
    fi
  fi
  if command -v codex >/dev/null 2>&1; then
    if codex mcp list 2>/dev/null | grep -q '^messaging-mcp\b'; then
      say "Codex: messaging-mcp is already registered."
    else
      codex mcp add messaging-mcp -- "$DIR/messaging-mcp" mcp >/dev/null &&
        say "Codex: registered messaging-mcp."
    fi
  fi
fi

# How each real message is approved. Asked, never assumed: it decides who says yes.
approval="${MESSAGING_MCP_APPROVAL:-}"
# /dev/tty can exist without a terminal behind it (no controlling terminal): try opening it.
if [ -z "$approval" ] && (: </dev/tty) 2>/dev/null; then
  printf '\nApprove each real message in your assistant'"'"'s own prompt (it shows the\ndestination and the whole message), instead of in a terminal? [Y/n] ' >/dev/tty
  answer=""
  read -r answer </dev/tty || answer=""
  case "$answer" in
    n* | N*) approval="terminal" ;;
    *) approval="client-prompt" ;;
  esac
fi
case "$approval" in
  client-prompt | terminal)
    if "$DIR/messaging-mcp" approval "$approval" >/dev/null 2>&1; then
      say "Approval: $approval."
    else
      say "Approval: this version cannot change it; it stays terminal. Install a newer release to use client-prompt."
    fi
    ;;
  *)
    say "Approval: terminal (the default). To approve in your assistant's prompt instead, run:"
    say "  $DIR/messaging-mcp approval client-prompt"
    ;;
esac

case ":$PATH:" in
  *":$DIR:"*) ;;
  *) say "Note: $DIR is not on your PATH. Add it to run messaging-mcp by name." ;;
esac

say ""
say "Done. Open a new Claude Code or Codex session and ask:"
say "  \"set up messaging-mcp and send me a hi on Telegram\""
