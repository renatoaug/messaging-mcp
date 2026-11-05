# Setup

Updated: 2026-11-05

> Build the binary, write a config, add the secrets, register the server in your MCP clients. Every step that creates account access is yours to run; nothing here does it for you. **The steps for real accounts are written but have not been run end to end yet.**

## Build

```sh
cargo build --release
./target/release/messaging-mcp --version
```

The binary is `target/release/messaging-mcp`. Copy it somewhere stable (for example `~/.local/bin/`) so client configs keep working after a rebuild. Release binaries will be attached to each GitHub release once the release pipeline has run.

## The config file

Start from `examples/messaging-mcp.personal.example.json`, copied **outside the repository**. Relative paths are resolved from the config file's folder, not from where a client was started. Unknown fields are an error.

| Field | What it is | Default |
|---|---|---|
| `mode` | `dry-run` (nothing reaches an account) or `live` | `dry-run` |
| `stateDir` | The ledger's folder, created 0700. Every client must use the same one | `.messaging-mcp-state` |
| `planTtlSeconds` | How long a plan, and its approval, stay valid (30–86400) | `600` |
| `maxSendsPerMinute` | Attempts per minute across all clients (1–60) | `6` |
| `recipients` | The allowed recipients ([below](#recipients)) | `[]` |
| `whatsapp.accountLabel` | Your own description of the paired account. Required for live | — |
| `whatsapp.baseUrl` | The bridge, `http://127.0.0.1:PORT/api` or `http://[::1]:PORT/api` only | `http://127.0.0.1:8080/api` |
| `whatsapp.tokenFile` | The bridge's token file | — |
| `whatsapp.timeoutMs` | 100–120000 | `15000` |
| `telegram.apiId` | From my.telegram.org | — |
| `telegram.apiHashFile` | A file holding the 32-character API hash | — |
| `telegram.sessionFile` | Written by `telegram-login` | — |
| `telegram.expectedUserId` | Your Telegram user ID; `telegram-login` prints it | — |
| `telegram.timeoutMs` | 100–120000 | `20000` |

Each MCP process reads the config once: restart the clients after editing it.

### Recipients

```json
{ "id": "tg-self", "label": "My Saved Messages", "channel": "telegram", "target": { "kind": "self" } }
{ "id": "ana", "label": "Ana", "channel": "whatsapp", "target": "5511977776666@s.whatsapp.net" }
```

`id` is lowercase letters, digits, `_` and `-`. A WhatsApp `target` is an exact JID: country code and number with no `+` or spaces, then `@s.whatsapp.net`, or a group's `…@g.us`. Telegram targets are in [03-telegram.md](03-telegram.md#targets). Rather than typing them, use `messaging-mcp allow`, which validates and stores them in the ledger, where every client sees them at once.

## Secrets

Each secret comes from an environment variable first, then from its file. Files must be regular files you own, mode 0600, not symlinks.

| Secret | Variable | File field |
|---|---|---|
| WhatsApp bridge token | `WHATSAPP_BRIDGE_TOKEN` | `whatsapp.tokenFile` |
| Telegram API ID | `TELEGRAM_API_ID` | `telegram.apiId` (not secret) |
| Telegram API hash | `TELEGRAM_API_HASH` | `telegram.apiHashFile` |
| Telegram session | `TELEGRAM_SESSION` | `telegram.sessionFile` |

Prefer files. A GUI app does not inherit your shell's variables, and other processes of your user can read them.

## Going live

1. Read [SECURITY.md](../../SECURITY.md).
2. Set up the channels: [03-telegram.md](03-telegram.md), [04-whatsapp.md](04-whatsapp.md).
3. Set `"mode": "live"` and restart the clients.
4. Run `messaging-mcp doctor`. It checks the config only; it does not prove a login or a connection.
5. Do the first live test in [06-testing.md](06-testing.md#the-first-live-test).

## Claude Code and Codex

Both start the same command: `/absolute/path/messaging-mcp mcp`, with `MESSAGING_MCP_CONFIG` set to the config's absolute path. That variable holds a path, not a secret.

**Claude Code**, private to the current project:

```sh
claude mcp add --scope local --env MESSAGING_MCP_CONFIG=/absolute/path/messaging-mcp.config.json --transport stdio messaging-mcp -- /absolute/path/messaging-mcp mcp
claude mcp get messaging-mcp
```

`--scope user` makes it available in every project. To have Claude Code ask before each call, add to your permissions:

```json
{ "permissions": { "ask": ["mcp__messaging-mcp__*"] } }
```

**Codex**, either the CLI or `~/.codex/config.toml`, not both:

```sh
codex mcp add messaging-mcp --env MESSAGING_MCP_CONFIG=/absolute/path/messaging-mcp.config.json -- /absolute/path/messaging-mcp mcp
```

```toml
[mcp_servers.messaging-mcp]
command = "/absolute/path/messaging-mcp"
args = ["mcp"]
default_tools_approval_mode = "prompt"
env_vars = ["WHATSAPP_BRIDGE_TOKEN", "TELEGRAM_API_ID", "TELEGRAM_API_HASH", "TELEGRAM_SESSION"]

[mcp_servers.messaging-mcp.env]
MESSAGING_MCP_CONFIG = "/absolute/path/messaging-mcp.config.json"
```

`env_vars` lists names to pass through, never values; keep only the ones you use. Check with `codex mcp list`.

A client asking before a tool call and you approving a plan in a terminal are different controls. The first is optional comfort; the second is the gate for live messages.

## Still open

- [ ] A release binary to download instead of building.
- [ ] Moving from the TypeScript version: its `bridge.config.json` fields are the same, but its Telegram session file is not compatible, and its state directory (`.bridge-state`) is not read.
