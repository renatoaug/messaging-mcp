# ADR-008: Our own WhatsApp bridge, on whatsmeow

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: supersedes [ADR-005](ADR-005-whatsapp-external-bridge.md); [design/04-whatsapp.md](../design/04-whatsapp.md), [ADR-007](ADR-007-agent-runs-setup.md)

## Context

[ADR-005](ADR-005-whatsapp-external-bridge.md) used a third-party bridge, the Go program from `verygoodplugins/whatsapp-mcp`. It holds the owner's WhatsApp session but does far more than sending: it stores the full message history, posts webhooks, downloads media, and prints its pairing code only to a terminal. With [ADR-007](ADR-007-agent-runs-setup.md), the assistant had to clone it, build it with Go and a C compiler, and run it under a pseudo-terminal to read the code. Claude Code's auto mode refused to run that setup, and the owner did not want a stranger's program holding the account.

## Decision

**WhatsApp goes through our own bridge, `messaging-mcp-whatsapp-bridge`, built from `whatsapp-bridge/` in this repository on `whatsmeow`, the most used WhatsApp Web library. It keeps the device session and nothing else (no history, no media, no webhooks), never logs message contents, listens only on `127.0.0.1` with a token, and serves three routes: `GET /api/health`, `GET /api/qr` and `POST /api/send`. It uses a pure-Go SQLite driver, so it builds without a C compiler; each release ships it next to `messaging-mcp`, so users need no Go.** `whatsapp_setup` starts the installed binary under our supervisor; it downloads and builds nothing.

## Alternatives considered

- **Keep the third-party bridge:** works, but stores much more than needed, needs Go and a C compiler on the user's machine, and puts someone else's code in charge of the account.
- **`whatsapp-rust` inside `messaging-mcp`:** one binary and no Go at all, but a 0.x library whose own README does not qualify macOS, and a second SQLite in the binary. Kept in mind for later.
- **The official WhatsApp Business/Cloud API:** official, but for companies, not a personal account.

## Consequences

- About 300 lines of Go to own, with offline tests (routes, token, recipients, a disconnected bridge, a failed send) run by `scripts/check.sh` and CI.
- The send contract is ours: 400, 401 and 409 (not connected) mean nothing was attempted; any other failure is uncertain. The response carries WhatsApp's message ID, now kept in the receipt.
- `whatsapp_setup` reads the paired account from `/api/health` instead of the bridge's database, and the QR code from `/api/qr` instead of a terminal drawing.
- `whatsmeow` is updated by us, at our pace, with its tests and a live pairing.
- A session paired by the old bridge is not reused: pairing again is needed once.
