# Messaging MCP

Claude Code, Codex and other MCP clients send messages through **your personal WhatsApp and Telegram accounts**, and you approve every real message before it leaves. MCP (Model Context Protocol) is how these assistants call outside tools. Messaging MCP is one local binary: an MCP server over stdio and a CLI, both on the same core and the same local ledger. The assistant also sets it up: you scan a QR code and say yes.

<p align="center">
  <a href="#running">📚 Get started</a> ·
  <a href="docs/design/01-architecture.md">🏗️ Architecture</a> ·
  <a href="docs/design/02-setup.md">⚙️ Setup</a> ·
  <a href="SECURITY.md">🔒 Security</a> ·
  <a href="docs/decisions/">🧭 Decisions</a>
</p>

**State:** Telegram works end to end with a real account (tested on 2026-11-05: QR login by the agent, approval in Claude Code's prompt, two messages delivered to Saved Messages). WhatsApp is built and tested offline only; its first live pairing is still to be done ([docs/design/06-testing.md](docs/design/06-testing.md)).

## What it does

- **Sends plain text** to recipients you allowed, on Telegram (as your own user, not a bot) and on WhatsApp (through a separate bridge program the assistant builds and starts).
- **Asks you first, every time.** The assistant prepares an exact message and shows it; your client then asks you to approve that destination and that text. Nothing leaves without your yes, and the yes never comes from the model.
- **Sets itself up from the conversation**: the assistant creates the Telegram app, shows you QR codes to scan, builds the WhatsApp bridge and allows recipients when you ask to message someone.
- **Searches your Telegram contacts and chats** to find the person you mean. Search only suggests.
- **Does not** read messages, search WhatsApp contacts (the bridge offers no way to), send media or formatting, or retry a send whose outcome is uncertain.

## Stack

Rust, one binary. MCP through the official Rust SDK (`rmcp`), Telegram through MTProto with `grammers`, WhatsApp through an external Go bridge over loopback HTTP, the ledger in SQLite. Why Rust: [ADR-001](docs/decisions/ADR-001-rust-single-binary.md). Why the assistant runs the setup: [ADR-007](docs/decisions/ADR-007-agent-runs-setup.md).

## Requirements

- macOS or Linux. Windows is not supported.
- A release binary, or [Rust](https://rustup.rs) stable to build one.
- For WhatsApp: `git`, [Go](https://go.dev) and a C compiler (Xcode command line tools on macOS). The assistant builds the bridge with them.
- To let the assistant create the Telegram app itself: a browser it can drive, such as the Claude in Chrome extension.

## Running

Build and try it with no account at all:

```sh
cargo build --release
cargo run --example demo
```

The demo prepares and simulates one message per channel: `simulated`, not `sent`.

Then install the binary and register it in your clients. It creates its config in `~/.config/messaging-mcp/` on first use:

```sh
install -m 755 target/release/messaging-mcp ~/.local/bin/messaging-mcp
claude mcp add --scope user messaging-mcp -- ~/.local/bin/messaging-mcp mcp
codex mcp add messaging-mcp -- ~/.local/bin/messaging-mcp mcp
```

Open a new session and ask: *"set up messaging-mcp and send me a hi on Telegram"*. The assistant follows `setup_status`. Before the first real message, turn on approval in your client: [docs/design/02-setup.md](docs/design/02-setup.md#approving-messages).

## Use it from your assistant

| You say | The assistant |
|---|---|
| "Set up messaging-mcp" | Creates the Telegram app, shows the QR code, builds and pairs WhatsApp |
| "Send Maria: I'll be 10 minutes late" | Finds Maria, allows her, prepares the message, shows it, and sends after your yes |
| "Switch to dry-run" | Stops reaching any account |

The tools: `setup_status`, `telegram_set_credentials`, `telegram_login`, `whatsapp_setup`, `whatsapp_status`, `whatsapp_stop`, `set_mode`, `search_contacts`, `allow_recipient`, `revoke_recipient`, `list_recipients`, `prepare_message`, `send_message`, `get_message_status`, `get_status`. There is no tool that approves a message.

## Test

```sh
scripts/check.sh   # format, clippy, licenses and advisories, unused deps, tests with an 80% floor, the demo
cargo test         # only the tests
```

The tests never reach a network beyond loopback, never read your real config or session files, and run with an empty environment. `git config core.hooksPath .githooks` turns on a pre-push hook that runs `scripts/check.sh`, the same as CI.

## Releases

Every merge to `main` is a release: [semantic-release](https://semantic-release.gitbook.io) reads the Conventional Commit titles since the last tag, bumps the version in `Cargo.toml` (a `feat` bumps the minor, anything else the patch), writes `CHANGELOG.md`, tags `vX.Y.Z` and publishes the GitHub release with binaries for macOS and Linux.

## Docs

Everything about how Messaging MCP works, runs and was decided is indexed in [`docs/`](docs/README.md). Code rules and the checks before calling a change done are in [`AGENTS.md`](AGENTS.md). We follow [Conventional Commits](https://www.conventionalcommits.org).

## License

[MIT](LICENSE). Unofficial: not affiliated with WhatsApp or Telegram. The WhatsApp bridge is a separate project under its own license; nothing from it is copied here.
