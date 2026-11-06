# Security

Messaging MCP can send messages as you. Message content, web pages, contact names and instructions from third parties never authorize a send: you name the recipient and the text you want sent.

## Defaults and controls

- **Dry-run by default.** No account connection is opened.
- **No listener of its own.** MCP runs over stdio. The WhatsApp bridge is reached only at a literal loopback address (`127.0.0.1` or `[::1]`) with a port and the `/api` path; redirects and environment proxies are refused.
- **An allowlist of recipients**, from the config file, added by the agent with `allow_recipient` when you ask to message someone, or added with `messaging-mcp allow` in a terminal. No wildcards, no matching by display name, no automatic expansion.
- **Your yes before every live message.** `send_message` has to repeat the recipient and the whole text, and the server refuses a mismatch. The yes comes from the one way you chose: the client's permission prompt for that call, which shows the destination and the text (`"approval": "client-prompt"`), MCP's approval dialog for a client that shows it (`dialog`), `messaging-mcp approve` in a terminal (`terminal`, the default), or your answer in the chat to the agent (`agent`). See [ADR-009](docs/decisions/ADR-009-approval-is-the-mode-the-owner-chose.md) and [ADR-010](docs/decisions/ADR-010-the-agent-can-be-the-approval.md). Approvals expire with the plan (10 minutes by default). See [ADR-007](docs/decisions/ADR-007-agent-runs-setup.md).
- **Setup by the agent, with your phone and passwords kept out of the chat.** You scan QR codes on your phone and type a Telegram 2FA password in a terminal (`messaging-mcp telegram-password`). The Telegram API hash goes straight to a private file and is never returned. Config writes are validated and atomic, in a 0700 folder.
- **Plans are bound to their context.** A fingerprint covers the mode, the recipient, the text and the sender's credentials, without exposing them. Changing any of them invalidates the plan. Telegram also checks the logged-in user at every send.
- **At most one attempt per plan.** The ledger records `sending` in a committed transaction (SQLite, `BEGIN IMMEDIATE`, WAL, `synchronous=FULL`) before the adapter runs. No automatic retries anywhere; Telegram uses a stable `random_id` and a sender pool with retries disabled.
- **Telegram sends literal text**: no entities, no link preview, never paid messages or paid flood skips.
- **Contact search is read-only**, live-only, and reads only your own contacts and chats, never the global directory. Access hashes are not shown to the agent.
- **Provider errors are never shown.** They could contain URLs, sessions, recipients or tokens. In MCP mode stdout is protocol only.

## Important limits

1. **`client-prompt` approval is only as strong as your client's prompt.** In Claude Code, keep `"ask": ["mcp__messaging-mcp__send_message"]` in your settings, and never choose "always allow" on that prompt. An auto-approving mode or an "always allow" lets the agent send without asking you. In Codex, keep `approval_mode = "prompt"` under `[mcp_servers.messaging-mcp.tools.send_message]`, and keep **Ask for approval**: with **Approve for me**, Codex's automatic reviewer allowed the prompt on its own in a test. `messaging-mcp approval client-prompt` sets both settings and repeats this warning.
2. **With `agent` approval, nothing outside the model checks a live send.** The agent is told to show each message and wait for your yes, but a mistake, or instructions hidden in a message or page it reads, could make it send to any recipient you allowed. Keep the allowed list short, or use `client-prompt` or `terminal`.
3. **A terminal check does not tell a person from an agent with a shell.** A process running as your user can also edit the config, the ledger and the code, read secrets and call the providers directly. Do not give an untrusted agent unrestricted access to the machine. Strong isolation needs a separate user or service that holds the credentials, with approval out of the agent's reach. This version does not build that.
4. **The paired WhatsApp account is reported by the bridge.** `whatsapp_setup` sets `accountLabel` from the number in `/api/health`. If you pair another account, generate a new bridge token. Never reuse a token across accounts.
5. **The Telegram session file is full access to your account.** Do not copy it between machines. This project registers no update handlers and exposes no reading, but that does not reduce what the credential can do.
6. **WhatsApp Web is unofficial for this use.** Our bridge links as a WhatsApp Web device through `whatsmeow`; WhatsApp can change it or restrict accounts. The bridge keeps only the device session, in `~/.config/messaging-mcp/whatsapp-bridge/store/` (0700), and its token there (0600). It stores no history and never logs message contents; `bridge.log` holds connection events only. Unlink it under WhatsApp → Linked devices if you stop using it.
7. **QR pages are written to `~/Downloads`** while you scan. Each turns into a result page when the login ends. A login QR is only useful to the device that created it, but delete the pages if you share that folder.
8. **Pending messages are stored in clear text** in the private ledger. File permissions are not encryption. Keep the state directory on a protected volume, out of synced folders and shared backups. Nothing deletes old plans yet.
9. **Accepted is not delivered.** Even after a timeout the provider may finish the send. There is no exactly-once delivery across the network.
10. **The bridge comes back on its own.** When an MCP client starts and the bridge was paired before but is not running (after a restart, for example), the server starts it again; it reconnects without a QR code. A bridge that never finished pairing is not started unasked.
11. **The rate limit is local** to a state directory. It does not replace the services' own rules. Do not use this for bulk messages, contact harvesting or to get around blocks.
12. **Config edits made by hand need a reconnect** of each MCP process (`/mcp` in Claude Code). Changes made through the setup tools and recipients added with `allow` take effect immediately.
13. **Files.** The agent can attach any file you can read, except hidden files, anything in a hidden folder (`~/.ssh`, `~/.config`, `.env`, `.git`) and messaging-mcp's own folders. Each file is copied when prepared and shown to you with its name, size, path and SHA-256 ([ADR-011](docs/decisions/ADR-011-files-are-copied-when-prepared.md)). Check the file before you say yes: with `agent` approval nothing else stands between an agent's mistake and the recipient.

## Secrets

Use regular files owned by you, mode 0600, inside a 0700 directory, outside the checkout. Linked secret files are refused. Never put the bridge token, the API hash, the session file, bridge databases, login codes, passwords or `.env` files in Git, attachments, logs or chats with agents.

Environment variables are supported, but other processes of the same user and some diagnostic tools can see them. Never put secrets on a command line or in MCP client configs. The Telegram logins never print the session; `messaging-mcp telegram-login` never overwrites an existing file.

## Recovery

- `failed`: fix the cause, check the chat when in doubt, and prepare a new message only if you still want it sent.
- `outcome_unknown`, or `sending` after a crash: look at the real chat. The project never reopens or resends that plan. Even if it was not sent, a new plan is a new authorization.
- `RESULT_NOT_PERSISTED`: the attempt happened but its result was not saved. The claim stays. Do not read it as a failure before sending.
- A compromised account or session: revoke the device in the official app's settings, stop the processes, replace the secrets, and review pending plans.

## Reporting a problem

Do not open a public issue for a security problem, and never include credentials or private content. Report it privately to the maintainer. A private reporting channel must be set up before the project is published.
