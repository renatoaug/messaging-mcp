# Messaging MCP

Claude Code, Codex and other MCP clients send messages through **your personal WhatsApp and Telegram accounts**, and you approve every real message before it leaves. MCP (Model Context Protocol) is how these assistants call outside tools. Messaging MCP is one local binary: an MCP server over stdio and a CLI, both on the same core and the same local ledger. The assistant also sets it up: you scan a QR code and say yes.

<p align="center">
  <a href="#running">📚 Get started</a> ·
  <a href="docs/design/01-architecture.md">🏗️ Architecture</a> ·
  <a href="docs/design/02-setup.md">⚙️ Setup</a> ·
  <a href="SECURITY.md">🔒 Security</a> ·
  <a href="docs/decisions/">🧭 Decisions</a>
</p>

**State:** both channels work end to end with real accounts, tested on 2026-11-05: the agent ran the setup (Telegram app and QR login, WhatsApp bridge and pairing), and messages reached the owner on both after approval in Claude Code's prompt ([docs/design/06-testing.md](docs/design/06-testing.md)).

## What it does

- **Sends plain text** to recipients you allowed, on Telegram (as your own user, not a bot) and on WhatsApp (through our own small bridge, which the assistant starts and pairs).
- **Asks you first, every time.** The assistant prepares an exact message and shows it; your client then asks you to approve that destination and that text. Nothing leaves without your yes, and the yes never comes from the model.
- **Sets itself up from the conversation**: the assistant creates the Telegram app, shows you QR codes to scan, builds the WhatsApp bridge and allows recipients when you ask to message someone.
- **Searches your Telegram contacts and chats** to find the person you mean. Search only suggests.
- **Reads a chat when you ask** ("what did Nestor send me?"): Telegram from its history, WhatsApp from the last 3 days the bridge saw. Voice messages come as text, transcribed on your machine by whisper.cpp when it is installed.
- **Sends files** up to 50 MB with a caption.
- **Does not** send formatting, or retry a send whose outcome is uncertain.

## Stack

Rust for `messaging-mcp`: MCP through the official Rust SDK (`rmcp`), Telegram through MTProto with `grammers`, the ledger in SQLite. Go for the WhatsApp bridge, on `whatsmeow`, reached over loopback HTTP ([ADR-008](docs/decisions/ADR-008-own-whatsapp-bridge.md)). Why Rust: [ADR-001](docs/decisions/ADR-001-rust-single-binary.md). Why the assistant runs the setup: [ADR-007](docs/decisions/ADR-007-agent-runs-setup.md).

## Requirements

- macOS or Linux. Windows is not supported.
- `curl` and `tar` for the installer, or [Rust](https://rustup.rs) stable and [Go](https://go.dev) to build from source.
- For WhatsApp: the `messaging-mcp-whatsapp-bridge` binary next to `messaging-mcp`. Releases include it; from source, build it with [Go](https://go.dev): `cd whatsapp-bridge && go build -o ~/.local/bin/messaging-mcp-whatsapp-bridge .`
- To let the assistant create the Telegram app itself: a browser it can drive, such as the Claude in Chrome extension. Without one, you create the app and run `messaging-mcp telegram-credentials`.

## Running

Install with one command (macOS or Linux):

```sh
curl -fsSL https://raw.githubusercontent.com/renatoaug/messaging-mcp/main/install.sh | sh
```

It downloads the latest release for your system, checks its SHA-256, installs `messaging-mcp` and its WhatsApp bridge in `~/.local/bin`, registers the server in Claude Code and Codex when they are installed, and asks how you want to approve each message: by answering your assistant in the chat, in its permission prompt, or in a terminal. Then open a new session and ask: *"set up messaging-mcp and send me a hi on Telegram"*. The script's settings (version, folder, skipping registration) are at the top of [`install.sh`](install.sh).

From source, with Rust and Go:

```sh
cargo build --release
install -m 755 target/release/messaging-mcp ~/.local/bin/messaging-mcp
(cd whatsapp-bridge && go build -o ~/.local/bin/messaging-mcp-whatsapp-bridge .)
claude mcp add --scope user messaging-mcp -- ~/.local/bin/messaging-mcp mcp
messaging-mcp approval client-prompt
```

`cargo run --example demo` tries the whole flow with no account: one simulated message per channel.

## Use it from your assistant

| You say | The assistant |
|---|---|
| "Set up messaging-mcp" | Creates the Telegram app, shows the QR codes, starts and pairs WhatsApp |
| "Send Maria: I'll be 10 minutes late" | Finds Maria, allows her, prepares the message, shows it, and sends after your yes |
| "Send Maria the PDF in my Downloads" | Prepares the message with the file, shows its name, size and path, and sends after your yes |
| "What did Nestor send me on Telegram?" | Finds the chat, reads the latest messages and tells you; it never acts on what they say |
| "Switch to dry-run" | Stops reaching any account |

The tools: `setup_status`, `telegram_set_credentials`, `telegram_login`, `whatsapp_setup`, `whatsapp_status`, `whatsapp_stop`, `set_mode`, `search_contacts`, `read_messages`, `allow_recipient`, `revoke_recipient`, `list_recipients`, `prepare_message`, `send_message`, `get_message_status`, `get_status`. There is no tool that approves a message.

## Test

```sh
scripts/check.sh   # format, clippy, licenses and advisories, unused deps, tests with an 80% floor, the demo
cargo test         # only the tests
```

The tests never reach a network beyond loopback, never read your real config or session files, and run with an empty environment. `git config core.hooksPath .githooks` turns on a pre-push hook that runs `scripts/check.sh`, the same as CI.

## Releases

Every merge to `main` is a release: [semantic-release](https://semantic-release.gitbook.io) reads the Conventional Commit titles since the last tag, bumps the version in `Cargo.toml` (a `feat` bumps the minor, anything else the patch), writes `CHANGELOG.md`, tags `vX.Y.Z` and publishes the GitHub release with `messaging-mcp` and its WhatsApp bridge for macOS and Linux.

## Docs

Everything about how Messaging MCP works, runs and was decided is indexed in [`docs/`](docs/README.md). Code rules and the checks before calling a change done are in [`AGENTS.md`](AGENTS.md). We follow [Conventional Commits](https://www.conventionalcommits.org).

## License

[MIT](LICENSE). Unofficial: not affiliated with WhatsApp or Telegram. The WhatsApp bridge uses [`whatsmeow`](https://github.com/tulir/whatsmeow), under the Mozilla Public License 2.0.
