# Testing

Updated: 2026-11-05

> An offline suite that covers the whole flow with fakes, a coverage floor, and a first live test that only the owner can run. **The offline suite passes on macOS; CI on Linux and macOS is written but has not run yet. The live test has not been done.**

## The offline suite

`cargo test` runs 38 tests in `tests/`, grouped by area:

| File | Covers |
|---|---|
| `service.rs` | The state machine: dry-run, approval, idempotency across restarts, the concurrent claim, terminal results, validation, expiry and context changes, the shared rate limit, the crash after the claim, identity rotation, the failed final write, search and allow/revoke |
| `adapters.rs` | WhatsApp against a local HTTP server (the contract, timeouts, odd bodies, rejections, no token); Telegram against a fake transport (authorization, timeout, close failure), the exact request, receipts, name cleaning |
| `identity.rs` | The sender fingerprint, credentials, the session file format, which contacts are offered |
| `config.rs` | Strict config, the loopback URL, private secret files, the state directory |
| `cli.rs` | The real binary: an MCP client over stdio, the CLI across restarts, terminal-only commands refusing a pipe |

The suite never leaves loopback, never reads your config or session files, and never reads inherited credentials: functions that read the environment take an `Env`, tests pass `&|_| None`, and the binary runs with a cleared environment.

## Checks and the floor

`scripts/check.sh` is what CI and the pre-push hook run: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check` (advisories, licenses, sources), `cargo machete` (unused dependencies), `cargo llvm-cov` failing under **80% of lines**, and the demo.

The floor leaves out `src/main.rs`, `src/adapters/telegram_client.rs` and `src/adapters/telegram_login.rs`: they only do something against a real account. Their logic that can run offline (the request, receipts, contact filtering, the session format, credentials) lives in `telegram.rs` and is covered. Today the measured lines are at 83%.

## The first live test

Only the owner runs this, in this order, stopping at the first surprise:

1. Telegram: `telegram-login`, set `expectedUserId`, `"mode": "live"`, then prepare a message to `tg-self` (Saved Messages), `approve` it, `send` it. Expect `sent` with a `providerMessageId`, and the message in Saved Messages, with `**asterisks**` shown literally.
2. Search: `messaging-mcp allow telegram "<a contact>" --id test-contact`, check the list is only your own contacts, then cancel or allow.
3. WhatsApp: pair the bridge, allow your own number, send one message to it.
4. From Claude Code or Codex: `get_status`, `list_recipients`, then a dry-run plan.

Record what happened in this note, and turn each surprise into a test.

## Still open

- [ ] The first run of CI and of the release pipeline.
- [ ] The live test above.
