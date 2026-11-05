# ADR-004: Telegram as the owner's own user, through MTProto

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/03-telegram.md](../design/03-telegram.md), [ADR-001](ADR-001-rust-single-binary.md)

## Context

A Telegram bot can only write to people who started a conversation with it, and messages arrive from the bot, not from the owner. Sending as the owner needs MTProto, Telegram's own protocol, with a user session. That session is full access to the account, and Telegram limits and may restrict accounts that misbehave.

## Decision

**Telegram sends as the owner's user through `grammers`, with: a session file created only by `messaging-mcp telegram-login`; the logged-in user checked against `expectedUserId` (and refused if it is a bot) before every send; one connection per send, closed afterwards; a sender pool with `NoRetries`; and `messages.sendMessage` with literal text, no entities, no link preview, no paid stars or flood skip, and a `random_id` derived from the plan ID.** The session file is our own JSON (the authorization keys and the home datacenter), mode 0600.

## Alternatives considered

- **Bot API:** simple and official, but sends as a bot and only to people who wrote to it first. Not what the owner asked for.
- **Teleproto (a TypeScript MTProto client):** what the first, TypeScript version used. Dropped with the move to Rust ([ADR-001](ADR-001-rust-single-binary.md)).
- **`grammers`'s SQLite session storage:** it pulls in `libsql`, a second SQLite build that conflicts with ours. The JSON file keeps one SQLite and a file we control.

## Consequences

- A send to the wrong account cannot happen silently: a session from another user or a bot fails before sending.
- Telegram can recognize a resent request by its `random_id`, on top of our own at-most-once claim.
- Sessions created by the TypeScript version do not work here; run `telegram-login` again.
- No updates are read and no handlers are registered, but the session could do so; that is why it stays private.
