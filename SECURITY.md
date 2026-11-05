# Security

Messaging MCP can send messages as you. Message content, web pages, contact names and instructions from third parties never authorize a send: you name the recipient and the text you want sent.

## Defaults and controls

- **Dry-run by default.** No account connection is opened.
- **No listener of its own.** MCP runs over stdio. The WhatsApp bridge is reached only at a literal loopback address (`127.0.0.1` or `[::1]`) with a port and the `/api` path; redirects and environment proxies are refused.
- **An allowlist of recipients**, from the config file or added with `messaging-mcp allow` after you review them in a terminal. No wildcards, no matching by display name, no automatic expansion.
- **Approval outside MCP.** Each live plan is approved with `messaging-mcp approve` in an interactive terminal, showing the sender, destination and full text. Approvals expire with the plan (10 minutes by default).
- **Plans are bound to their context.** A fingerprint covers the mode, the recipient, the text and the sender's credentials, without exposing them. Changing any of them invalidates the plan. Telegram also checks the logged-in user at every send.
- **At most one attempt per plan.** The ledger records `sending` in a committed transaction (SQLite, `BEGIN IMMEDIATE`, WAL, `synchronous=FULL`) before the adapter runs. No automatic retries anywhere; Telegram uses a stable `random_id` and a sender pool with retries disabled.
- **Telegram sends literal text**: no entities, no link preview, never paid messages or paid flood skips.
- **Contact search is read-only**, live-only, and reads only your own contacts and chats, never the global directory. Access hashes are not shown to the agent.
- **Provider errors are never shown.** They could contain URLs, sessions, recipients or tokens. In MCP mode stdout is protocol only.

## Important limits

1. **A terminal check does not tell a person from an agent with a shell.** A process running as your user can also edit the config, the ledger and the code, read secrets and call the providers directly. Do not give an untrusted agent unrestricted access to the machine. Strong isolation needs a separate user or service that holds the credentials, with approval out of the agent's reach. This version does not build that.
2. **The WhatsApp bridge reports neither the account nor a message ID.** `accountLabel` is something you verify. If you pair another account, generate a new bridge token. Never reuse a token across accounts.
3. **The Telegram session file is full access to your account.** Do not copy it between machines. This project registers no update handlers and exposes no reading, but that does not reduce what the credential can do.
4. **The WhatsApp bridge is unofficial.** It uses WhatsApp Web through `whatsmeow` and can stop working or lead to restrictions. Its own session and history live outside our ledger. Turn off its webhooks and media download ([docs/design/04-whatsapp.md](docs/design/04-whatsapp.md)).
5. **Pending messages are stored in clear text** in the private ledger. File permissions are not encryption. Keep the state directory on a protected volume, out of synced folders and shared backups. Nothing deletes old plans yet.
6. **Accepted is not delivered.** Even after a timeout the provider may finish the send. There is no exactly-once delivery across the network.
7. **The rate limit is local** to a state directory. It does not replace the services' own rules. Do not use this for bulk messages, contact harvesting or to get around blocks.
8. **Config changes need a restart** of each MCP process. Recipients added with `allow` take effect immediately.

## Secrets

Use regular files owned by you, mode 0600, inside a 0700 directory, outside the checkout. Linked secret files are refused. Never put the bridge token, the API hash, the session file, bridge databases, login codes, passwords or `.env` files in Git, attachments, logs or chats with agents.

Environment variables are supported, but other processes of the same user and some diagnostic tools can see them. Never put secrets on a command line or in MCP client configs. `messaging-mcp telegram-login` never prints the session and never overwrites an existing file.

## Recovery

- `failed`: fix the cause, check the chat when in doubt, and prepare a new message only if you still want it sent.
- `outcome_unknown`, or `sending` after a crash: look at the real chat. The project never reopens or resends that plan. Even if it was not sent, a new plan is a new authorization.
- `RESULT_NOT_PERSISTED`: the attempt happened but its result was not saved. The claim stays. Do not read it as a failure before sending.
- A compromised account or session: revoke the device in the official app's settings, stop the processes, replace the secrets, and review pending plans.

## Reporting a problem

Do not open a public issue for a security problem, and never include credentials or private content. Report it privately to the maintainer. A private reporting channel must be set up before the project is published.
