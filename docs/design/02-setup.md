# Setup

Updated: 2026-11-05

> The assistant runs the setup through MCP tools; the owner scans QR codes, reads one login code, and approves each real message in their client. **Telegram was set up this way end to end on 2026-11-05. The WhatsApp steps run offline in tests but have not paired a real phone yet.** Record: [ADR-007](../decisions/ADR-007-agent-runs-setup.md).

## Install and register

```sh
cargo build --release
install -m 755 target/release/messaging-mcp ~/.local/bin/messaging-mcp
claude mcp add --scope user messaging-mcp -- ~/.local/bin/messaging-mcp mcp
codex mcp add messaging-mcp -- ~/.local/bin/messaging-mcp mcp
```

No variable is needed: without `--config` or `MESSAGING_MCP_CONFIG`, the binary uses `~/.config/messaging-mcp/messaging-mcp.config.json` (or `$XDG_CONFIG_HOME/messaging-mcp/`) and creates it in dry-run on first use, in a 0700 folder. After a new binary, reconnect the server (`/mcp` in Claude Code).

## What the assistant does

`setup_status` reports what is configured and lists the next steps in order. The assistant follows them:

| Step | Tool | What the owner does |
|---|---|---|
| 1. Create the Telegram app at my.telegram.org | a browser the assistant drives, then `telegram_set_credentials` | Signs in on the page or reads the code Telegram sends to their app |
| 2. Log in to Telegram | `telegram_login`, called until it says `logged_in` | Scans the QR page (Telegram > Settings > Devices > Link Desktop Device). With a 2FA password: runs `messaging-mcp telegram-password` and types it |
| 3. Build, start and pair WhatsApp | `whatsapp_setup`, then `whatsapp_status` | Scans the QR page (WhatsApp > Settings > Linked devices > Link a device) |
| 4. Go live | `set_mode` with `live` | Turns on approval (below) |
| 5. Allow people | `search_contacts`, `allow_recipient` | Asks to message someone |

The QR pages are written to `~/Downloads` (`messaging-mcp-telegram-login.html`, `messaging-mcp-whatsapp-pairing.html`), reload every 5 seconds to pick up a new code, and turn into a result page when the login ends. `cargo run --example preview_pages` writes every state of them to look at.

The Telegram API hash is written to `telegram-api-hash` (0600) and never returned. A login whose result was not recorded (for example after a reconnect) is recovered by `telegram_login`: it asks Telegram whose saved session it is instead of asking for a new scan.

## Approving messages

`send_message` repeats the plan's recipient and whole text; the server refuses a mismatch. Who says yes, in order:

1. **The client's approval dialog** (MCP elicitation), when the client offers one. Claude Code did not on 2026-11-05.
2. **The client's permission prompt**, with `"approval": "client-prompt"` in the config. The prompt shows the destination and the text. Keep it on:
   - Claude Code, in `~/.claude/settings.json`: `{ "permissions": { "ask": ["mcp__messaging-mcp__send_message"] } }`. This also holds in auto mode. Never choose "always allow" on this prompt.
   - Codex, in `~/.codex/config.toml`: `default_tools_approval_mode = "prompt"` under `[mcp_servers.messaging-mcp]`.
3. **The terminal**, the default (`"approval": "terminal"`): `messaging-mcp approve <planId>`.

## The config file

The setup tools write it; editing by hand works too, followed by a reconnect. Relative paths are resolved from the file's folder. Unknown fields are an error.

| Field | What it is | Default |
|---|---|---|
| `mode` | `dry-run` (nothing reaches an account) or `live` | `dry-run` |
| `approval` | `terminal` or `client-prompt` (see above) | `terminal` |
| `stateDir` | The ledger's folder, created 0700. Every client must use the same one | `.messaging-mcp-state` (`state` in the created config) |
| `planTtlSeconds` | How long a plan, and its approval, stay valid (30–86400) | `600` |
| `maxSendsPerMinute` | Attempts per minute across all clients (1–60) | `6` |
| `recipients` | Recipients written by hand; `allow_recipient` stores others in the ledger | `[]` |
| `whatsapp.accountLabel` | The paired account; `whatsapp_setup` sets it from the bridge | — |
| `whatsapp.baseUrl` | The bridge, `http://127.0.0.1:PORT/api` or `http://[::1]:PORT/api` only | `http://127.0.0.1:8080/api` |
| `whatsapp.tokenFile` | The bridge's token file | — |
| `whatsapp.timeoutMs` | 100–120000 | `15000` |
| `telegram.apiId` | From my.telegram.org | — |
| `telegram.apiHashFile` | The file with the 32-character API hash | — |
| `telegram.sessionFile` | Written by the login | — |
| `telegram.expectedUserId` | Your Telegram user ID; the login sets it | — |
| `telegram.timeoutMs` | 100–120000 | `20000` |

## Secrets

Each secret comes from an environment variable first, then from its file. Files must be regular files you own, mode 0600, not symlinks.

| Secret | Variable | File field |
|---|---|---|
| WhatsApp bridge token | `WHATSAPP_BRIDGE_TOKEN` | `whatsapp.tokenFile` |
| Telegram API ID | `TELEGRAM_API_ID` | `telegram.apiId` (not secret) |
| Telegram API hash | `TELEGRAM_API_HASH` | `telegram.apiHashFile` |
| Telegram session | `TELEGRAM_SESSION` | `telegram.sessionFile` |

## Without the assistant

Every step also works from a terminal: `messaging-mcp telegram-login` (phone code instead of QR), `allow whatsapp|telegram`, `approve`, `revoke`, `whatsapp-bridge-stop`. `messaging-mcp --help` lists them.

## Still open

- [ ] Entering the Telegram credentials without a browser the assistant can drive: a `messaging-mcp telegram-credentials` command that asks for them hidden, so the hash never goes through the chat.
- [ ] Moving from the TypeScript version: its Telegram session file is not compatible, and its state directory (`.bridge-state`) is not read.
