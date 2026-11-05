# Decision records

The project's decision log: short, dated, immutable records of structural decisions. They answer *"why is it like this?"* months later, and keep settled discussions from being reopened by someone who did not know they were settled.

Two kinds, same format:

- **ADR** (Architecture Decision Record) — technical choices: `ADR-NNN-slug.md`
- **PDR** (Product Decision Record) — product direction and scope: `PDR-NNN-slug.md`

The depth behind a record (diagrams, every table, every setting) lives in [`docs/design/`](../design/). Records are the memory; the design notes are the depth.

## Rules

1. **Records are immutable.** Reversing a decision means writing a *new* record and marking the old one `Superseded by ADR-NNN` in its Status line — never editing history.
2. **Write it in the same pull request.** A change that resolves a structural question ships with its record, written while the context is loaded.
3. **Keep it to half a page.** Four sections: Context, Decision, Alternatives considered, Consequences. If it needs more, link a design note or an issue.
4. **Number sequentially** within each prefix.
5. **State the decision as a rule someone can follow**, not as a summary of a discussion.

## Template

```markdown
# ADR-NNN: <Title>

- **Status**: Accepted | Superseded by ADR-NNN
- **Date**: YYYY-MM-DD
- **Related**: design notes, issues, other records

## Context
What was the situation and the forces at play? (2–5 sentences)

## Decision
What we decided, stated as a rule someone can follow.

## Alternatives considered
What else was on the table and why it lost. (bullets)

## Consequences
What this makes easier, what it makes harder, what to watch. (bullets)
```

## Index

| Record | Title | Status |
|---|---|---|
| [ADR-001](ADR-001-rust-single-binary.md) | One Rust binary for the MCP server and the CLI | Accepted |
| [ADR-002](ADR-002-plans-sent-at-most-once.md) | Every message is a durable plan, attempted at most once | Accepted |
| [ADR-003](ADR-003-human-steps-outside-mcp.md) | Approving, allowing and logging in happen only in a terminal | Superseded by ADR-007 |
| [ADR-004](ADR-004-telegram-personal-mtproto.md) | Telegram as the owner's own user, through MTProto | Accepted |
| [ADR-005](ADR-005-whatsapp-external-bridge.md) | WhatsApp through one external bridge the owner runs | Superseded by ADR-008 |
| [ADR-006](ADR-006-contact-search-suggests.md) | Contact search suggests; only the owner adds a recipient | Partly superseded by ADR-007 |
| [ADR-007](ADR-007-agent-runs-setup.md) | The agent runs the setup; the owner approves each live message in their client | Accepted |
| [ADR-008](ADR-008-own-whatsapp-bridge.md) | Our own WhatsApp bridge, on whatsmeow | Accepted |
