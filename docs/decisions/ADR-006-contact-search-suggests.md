# ADR-006: Contact search suggests; only the owner adds a recipient

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/03-telegram.md](../design/03-telegram.md#finding-and-allowing-people), [ADR-003](ADR-003-human-steps-outside-mcp.md)

## Context

The first version could only send to recipients written by hand in the config file, and a Telegram user needs an ID and an access hash nobody knows by heart. In practice it could only send to the owner's own Saved Messages. Letting the agent pick recipients by name, on the other hand, means a wrong or malicious match sends to the wrong person.

## Decision

**`search_contacts` (MCP) and `allow telegram` (CLI) search the owner's own Telegram contacts and chats (`contacts.search`, `my_results` only), read-only and in live mode only. Results show a name, kind and username, never the access hash. A result becomes a recipient only when the owner runs `messaging-mcp allow` in a terminal and confirms it; allowed recipients live in the ledger and every client sees them at once. `revoke` removes them.** WhatsApp recipients are allowed by exact number, since the bridge has no search.

## Alternatives considered

- **Let `prepare_message` take a name and resolve it:** one step fewer, but a name is not an identity, and the agent would decide who receives the message.
- **Write allowed recipients into the config file:** keeps one source, but needs every MCP process restarted and a program rewriting a file the owner edits.
- **Include Telegram's global search results:** finds strangers by username. Not needed to write to people the owner knows; excluded.

## Consequences

- The owner can message anyone in their Telegram, after one terminal step per new person.
- Contact names come from other people and are treated as data: control characters are stripped and they are shown as JSON.
- Search connects to Telegram and counts against its limits; it is capped at 20 results and runs only when asked.
- Reading messages stays out of scope; it would need its own record about untrusted content.
