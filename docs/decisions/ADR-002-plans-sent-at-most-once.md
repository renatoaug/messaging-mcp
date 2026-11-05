# ADR-002: Every message is a durable plan, attempted at most once

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/01-architecture.md](../design/01-architecture.md#the-life-of-a-plan), [design/05-storage.md](../design/05-storage.md)

## Context

Several clients (Claude Code, Codex, the CLI) can run at once on the same machine, each in its own process. A network send can time out after the provider already accepted it, so "it failed, try again" can mean a message sent twice to a real person. Neither WhatsApp's bridge nor Telegram give us an exactly-once guarantee.

## Decision

**A message is first a plan in the shared SQLite ledger, identified by the client's idempotency key. `send` commits the plan as `sending` before calling the provider, and a plan that reached `sending` is never attempted again, by anything.** An uncertain result is `outcome_unknown`, and only a person looking at the chat can resolve it. All clients on a machine use the same config and state directory.

## Alternatives considered

- **Retry on timeout:** the common default, and the way duplicates happen. Rejected.
- **A lease that expires and lets another client take over a stuck `sending`:** recovers from crashes, but turns "maybe sent" into "sent twice". Rejected.
- **A remote queue or dispatcher (Redis, a server):** shared state across machines, at the cost of a service to run and secure. Out of scope for a local tool.

## Consequences

- The same plan is never sent twice, across restarts and concurrent clients. Tests cover the crash after the claim, the concurrent claim and the failed final write (`RESULT_NOT_PERSISTED`).
- A crash leaves a plan blocked in `sending` on purpose; the owner checks the chat.
- A new idempotency key is a new intent: two keys can send the same text twice, which is sometimes what the owner wants.
- The ledger must stay on a local disk; SQLite over a network share is not supported.
