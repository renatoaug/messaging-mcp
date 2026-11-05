# Messaging MCP

Claude Code, Codex and other MCP clients send messages through **your personal WhatsApp and Telegram accounts**, with you approving every real message in your own terminal. MCP (Model Context Protocol) is how these assistants call outside tools. Messaging MCP is one local binary: an MCP server over stdio and a CLI, both on the same core and the same local ledger.

<p align="center">
  <a href="#running">📚 Get started</a> ·
  <a href="docs/design/01-architecture.md">🏗️ Architecture</a> ·
  <a href="docs/design/02-setup.md">⚙️ Setup</a> ·
  <a href="SECURITY.md">🔒 Security</a> ·
  <a href="docs/decisions/">🧭 Decisions</a>
</p>

**State:** the whole flow runs and is tested offline, in dry-run (the default, where nothing reaches an account). **No real account has been connected yet**: the first live send, the Telegram login and the contact search against Telegram are still to be tried by hand ([docs/design/06-testing.md](docs/design/06-testing.md#the-first-live-test)).

## What it does

- **Sends plain text** to recipients you allowed, on WhatsApp (through a separate bridge program) and Telegram (as your own user, not a bot).
- **Plans before sending**: an assistant prepares an exact message; in live mode you approve it with `messaging-mcp approve` in a terminal; only then can it be sent, and never twice.
- **Searches your Telegram contacts and chats** so you can allow a new recipient with `messaging-mcp allow telegram "Maria"`. The search only suggests; adding is yours.
- **Does not**: read messages, search WhatsApp contacts (the bridge offers no way to), send media, log in on its own, start the WhatsApp bridge, or retry a send whose outcome is uncertain.

## Stack

Rust, one binary. MCP through the official Rust SDK (`rmcp`), Telegram through MTProto with `grammers`, WhatsApp through an external Go bridge over loopback HTTP, the ledger in SQLite. Why Rust: [ADR-001](docs/decisions/ADR-001-rust-single-binary.md).

## Requirements

- [Rust](https://rustup.rs) stable, installed with `rustup` (the repository's `rust-toolchain.toml` picks the version). Only to build; a release binary needs nothing.
- macOS or Linux. Windows is not supported.
- For WhatsApp: [Go](https://go.dev) to build the bridge, which runs separately ([docs/design/04-whatsapp.md](docs/design/04-whatsapp.md)).
- For Telegram: an `api_id` and `api_hash` from [my.telegram.org](https://my.telegram.org) ([docs/design/03-telegram.md](docs/design/03-telegram.md)).

## Running

Try it with no account at all:

```sh
cargo build --release
cargo run --example demo
./target/release/messaging-mcp doctor --config examples/messaging-mcp.config.json
./target/release/messaging-mcp prepare --config examples/messaging-mcp.config.json --recipient tg-self --text 'Hello, local test.' --key demo_intent_001
```

`prepare` prints a plan with an `id`. Send it with `messaging-mcp send <id> --config examples/messaging-mcp.config.json`: the result is `simulated`, not `sent`.

Connecting real accounts, every setting and how to register the server in Claude Code and Codex: [docs/design/02-setup.md](docs/design/02-setup.md).

## Use it from your assistant

Register the binary as a stdio MCP server, with `MESSAGING_MCP_CONFIG` pointing to your config file:

```sh
claude mcp add --scope local --env MESSAGING_MCP_CONFIG=/absolute/path/messaging-mcp.config.json --transport stdio messaging-mcp -- /absolute/path/messaging-mcp mcp
codex mcp add messaging-mcp --env MESSAGING_MCP_CONFIG=/absolute/path/messaging-mcp.config.json -- /absolute/path/messaging-mcp mcp
```

The tools are `get_status`, `list_recipients`, `search_contacts`, `prepare_message`, `get_message_status` and `send_message`. There is no tool to log in, approve or allow: those stay in your terminal. Details and permissions: [docs/design/02-setup.md](docs/design/02-setup.md#claude-code-and-codex).

## Test

```sh
scripts/check.sh   # format, clippy, licenses and advisories, unused deps, tests with an 80% floor, the demo
cargo test         # only the tests
```

The tests never reach a network beyond loopback, never read your real config or session files, and run with an empty environment. `git config core.hooksPath .githooks` turns on a pre-push hook that runs `scripts/check.sh`, the same as CI.

## Releases

Every merge to `main` is a release: [semantic-release](https://semantic-release.gitbook.io) reads the Conventional Commit titles since the last tag, bumps the version in `Cargo.toml` (a `feat` bumps the minor, anything else the patch), writes `CHANGELOG.md`, tags `vX.Y.Z` and publishes the GitHub release with binaries for macOS and Linux. This pipeline is written but has not run yet.

## Docs

Everything about how Messaging MCP works, runs and was decided is indexed in [`docs/`](docs/README.md). Code rules and the checks before calling a change done are in [`AGENTS.md`](AGENTS.md). We follow [Conventional Commits](https://www.conventionalcommits.org).

## License

[MIT](LICENSE). The WhatsApp bridge is a separate project under its own license; nothing from it is copied here.
