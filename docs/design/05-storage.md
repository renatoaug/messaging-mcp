# Storage

Updated: 2026-11-05

> One SQLite file, `plans.sqlite`, in `stateDir`, shared by every process on the machine. It holds the plans and the recipients added with `allow`. Record: [ADR-002](../decisions/ADR-002-plans-sent-at-most-once.md).

## Rules

- Tables are singular, columns snake_case. Timestamps are milliseconds since the epoch, in columns ending in `_at`.
- The schema changes only by adding an entry to `MIGRATIONS` in `src/store.rs`. `PRAGMA user_version` records how many have run. An old entry is never edited.
- Every write runs in `BEGIN IMMEDIATE`, so two processes never interleave. A process waits up to 5 s for another's lock, then answers `STATE_BUSY`.
- `journal_mode=WAL` and `synchronous=FULL`: a committed `sending` survives a crash or a power cut.
- `stateDir` must be a directory you own with mode 0700, not a symlink; `plans.sqlite` a regular file with mode 0600, opened without following symlinks. Anything else is `STATE_PERMISSIONS`.
- SQLite is compiled into the binary (`rusqlite` with `bundled`), version 3.54 today. The file must stay on a local disk.

## The config folder

Next to the config, in `~/.config/messaging-mcp/` (0700), the setup tools keep: `telegram-api-hash` and `telegram-session.json` (0600), `telegram-pending-session.json` while a login waits for a 2FA password, and `whatsapp-bridge/` with `store/` (the bridge's device session and token), `bridge.log` and `bridge.pid`.

## `plan`

One row per idempotency key.

| Column | What it is |
|---|---|
| `id` | `msg_` + 32 hex characters of the key's SHA-256 |
| `mode` | `dry-run` or `live`, when prepared |
| `recipient_id`, `recipient_label` | The recipient, as allowed when prepared |
| `sender` | Who it sends as, for the approval screen (`Telegram user:777`) |
| `channel`, `target` | `whatsapp` or `telegram`, and the destination as shown (`user:42`) |
| `text` | The exact message, **in clear text** |
| `fingerprint` | SHA-256 of mode, recipient, text and the sender's credentials |
| `status` | `pending`, `approved`, `sending`, `sent`, `simulated`, `failed` or `outcome_unknown` |
| `created_at`, `expires_at` | When prepared, and when it stops being sendable |
| `approved_at`, `attempted_at`, `completed_at` | Set by approve, by the claim, and by the result |
| `receipt_accepted`, `receipt_provider_message_id` | The provider's acceptance and its message ID, when it gave one |
| `error_code`, `error_message` | Our own code and message for `failed` or `outcome_unknown` |

`attempted_at` is indexed: the rate limit counts the rows attempted in the last 60 s.

## `recipient`

One row per recipient added with `allow`. Recipients in the config file are not stored here; an ID can be in only one of the two.

| Column | What it is |
|---|---|
| `id`, `label`, `channel` | As in the config |
| `target` | A WhatsApp JID, or a Telegram peer as JSON (`{"kind":"user","id":"42","accessHash":"-7"}`) |
| `created_at` | When it was allowed |

## Still open

- [ ] Retention: plans, with their text, are never deleted.
- [ ] Encryption at rest: none beyond file permissions.
