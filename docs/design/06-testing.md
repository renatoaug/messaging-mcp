# Testing

Updated: 2026-11-05

> An offline suite that covers the whole flow with fakes, a coverage floor, and live tests that only the owner can run. **The offline suite passes on macOS. The release pipeline ran (v0.1.1, four binaries); the pull request checks on Linux and macOS have not run yet, as no pull request was opened. Both channels passed their live test on 2026-11-05.**

## The offline suite

`cargo test` runs 59 tests in `tests/`, grouped by area:

| File | Covers |
|---|---|
| `service.rs` | The state machine: dry-run, approval, idempotency across restarts, the concurrent claim, terminal results, validation, expiry and context changes, the shared rate limit, the crash after the claim, identity rotation, the failed final write, search and allow/revoke, the approval text |
| `approval.rs` | The real binary with a client that answers the approval dialog: each mode (`dialog` approved and declined, `client-prompt` without a second question even when the client declares the dialog, `terminal`), no dialog, and a send that does not repeat the prepared message |
| `adapters.rs` | WhatsApp against a local HTTP server (the contract, timeouts, odd bodies, rejections, no token); Telegram against a fake transport (authorization, timeout, close failure), the exact request, receipts, name cleaning |
| `identity.rs` | The sender fingerprint, credentials, the session file format, which contacts are offered |
| `setup.rs` | The default config, validated atomic writes, private files, credentials, the setup status and next steps, the login reading the file and not a stale copy, QR codes read from the bridge's drawing, the pages |
| `config.rs` | Strict config, the loopback URL, private secret files, the state directory |
| `cli.rs` | The real binary: an MCP client over stdio (including the setup tools and the config reload), the CLI across restarts, terminal-only commands refusing a pipe, the config created on first use |
| `whatsapp-bridge/server_test.go` | The Go bridge's routes: the token on every route, health and the paired account, the QR code, one exact send, invalid requests sending nothing, a disconnected bridge attempting nothing, a failed send never reported as not sent |

The suite never leaves loopback, never reads your config or session files, and never reads inherited credentials: functions that read the environment take an `Env`, tests pass `&|_| None`, and the binary runs with a cleared environment.

## Checks and the floor

`scripts/check.sh` is what CI and the pre-push hook run: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check` (advisories, licenses, sources), `cargo machete` (unused dependencies), `cargo llvm-cov` failing under **80% of lines**, and the demo.

`scripts/check.sh` also runs `gofmt`, `go vet` and `go test` on the bridge.

The floor leaves out the files that only do something against a real account, a phone or a network: `src/main.rs`, `src/adapters/telegram_client.rs`, `src/adapters/telegram_login.rs`, `src/setup/telegram_qr.rs`, `src/setup/bridge_process.rs` and `src/setup/whatsapp_flow.rs`. Their logic that can run offline (the request, receipts, contact filtering, the session format, credentials, QR reading, the pages) lives elsewhere and is covered. Today the measured lines are at 81%.

`cargo run --example preview_pages` writes every state of the setup pages to `~/Downloads`, to look at after changing them.

## The first live test

Telegram, 2026-11-05, with Claude Code: the agent created the app at my.telegram.org through the browser, logged in by QR, and sent two messages to Saved Messages after approval in Claude Code's permission prompt (`client-prompt`). Surprises, each now fixed with a test: the QR page vanished on success instead of showing it; a second `telegram_login` discarded the first one's result; the login trusted the server's stale copy of the config; Claude Code offers no approval dialog (MCP elicitation), which led to `client-prompt` ([ADR-007](../decisions/ADR-007-agent-runs-setup.md)).

WhatsApp, 2026-11-05, with Claude Code, through our own bridge ([ADR-008](../decisions/ADR-008-own-whatsapp-bridge.md)): `whatsapp_setup` started it, the pairing page worked, the first attempt timed out before the scan and the second paired; the agent allowed the owner's own number and sent one message after approval. Surprises, now fixed: a pairing attempt lasts only about 2.5 minutes, so the supervisor now starts it again up to 4 times; `whatsapp_status` returned at once instead of waiting for the scan. Before this, Claude Code's auto mode refused to let the agent clone and build the third-party bridge, which led to our own.

Contact search, 2026-11-05: Telegram's `contacts.search` returned none of the owner's address book contacts, only global directory results, so search now filters the address book locally (tested offline in `identity.rs`).

Codex, 2026-11-05, in the ChatGPT desktop app: its permission prompt showed `planId`, `recipientId` and `text`. Surprises: Codex declares MCP's approval dialog but declined it unseen, so every send was refused after the owner allowed the prompt; and with "Approve for me" its reviewer allowed the prompt on its own. Hence [ADR-009](../decisions/ADR-009-approval-is-the-mode-the-owner-chose.md): one approval mode, no second confirmation, and a warning to keep "Ask for approval".

What is left, in this order, stopping at the first surprise:

1. Codex: a send with `client-prompt` after the fix, approved in its prompt.

Record what happened in this note, and turn each surprise into a test.

## Still open

- [x] The first run of the release pipeline → v0.1.1 with four binaries.
- [ ] The first run of the pull request checks.
- [x] Telegram live → 2026-11-05.
- [x] WhatsApp live → 2026-11-05.
- [ ] The rest of the live test above.
