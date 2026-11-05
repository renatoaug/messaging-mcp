# ADR-009: A live message is approved by the one mode the owner chose

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: refines [ADR-007](ADR-007-agent-runs-setup.md); [design/02-setup.md](../design/02-setup.md#approving-messages), [SECURITY.md](../../SECURITY.md)

## Context

[ADR-007](ADR-007-agent-runs-setup.md) tried MCP's approval dialog (elicitation) first whenever a client declared it, then the client's permission prompt, then the terminal. The live test in Codex on 2026-11-05 showed two problems. Codex declares the dialog but declined it without showing anything, so after the owner allowed `send_message` in Codex's permission prompt, the send was refused. And with Codex's "Approve for me", its automatic reviewer allowed the permission prompt on its own; only that unseen dialog stopped the send.

## Decision

**The `approval` setting names exactly one way a live message is approved, and nothing else is tried: `client-prompt` (the client's permission prompt for `send_message`, which shows the destination and the whole text), `dialog` (MCP's approval dialog, for clients that really show it) or `terminal` (`messaging-mcp approve`, the default).** `client-prompt` is a person's yes only while a person answers the prompt: in Codex "Ask for approval", never "Approve for me"; in either client, never "always allow". `messaging-mcp approval client-prompt` says so when it runs.

## Alternatives considered

- **Keep asking for the dialog whenever a client declares it:** it blocked the reviewer in Codex, but it also blocked the owner, so live sends from Codex were impossible.
- **Detect an automatic reviewer from the server:** MCP tells the server nothing about who answered a prompt.
- **Require the terminal for Codex:** safe, but it brings back the round trip ADR-007 removed.

## Consequences

- Codex and Claude Code both send after one yes in their own prompt.
- The safety of `client-prompt` rests on the client's settings, which the owner controls: an automatic reviewer or an always-allow lets the agent send without asking. The docs and the `approval` command warn about it.
- `dialog` stays available, tested, for a client that shows it.
