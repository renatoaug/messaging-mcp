# ADR-010: The owner can let the agent's chat confirmation be the approval

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: extends [ADR-009](ADR-009-approval-is-the-mode-the-owner-chose.md); [design/02-setup.md](../design/02-setup.md#approving-messages), [SECURITY.md](../../SECURITY.md)

## Context

With `client-prompt`, every live message asks twice: the agent shows the message and asks in the chat, then the client's permission prompt asks again. The owner wanted one yes, given in the chat. Other open-source messaging MCP servers mostly send on call and leave confirmation to the agent and the client; at most they split the send into a draft and a confirm call.

## Decision

**A fourth `approval` value, `agent`: `send_message` sends a live plan as soon as it is called, after the usual check that it repeats the prepared recipient and whole text. The server's instructions and the tool's description tell every agent, in every mode, to show the destination and the whole message, ask, end its turn, and call `send_message` only after the user's yes in their own words.** `messaging-mcp approval agent` removes the Claude Code ask rule and Codex's `approval_mode = "prompt"` that `client-prompt` added, and warns about what the owner gives up. It is never the default, and no MCP tool can choose it.

## Alternatives considered

- **Keep `client-prompt`:** one more click per message, which the owner did not want.
- **A `confirmation` argument the agent fills with the user's words:** the agent writes it, so it proves nothing the call itself does not.
- **Remove the other modes:** they stay for owners who want a check outside the model.

## Consequences

- Live messages go out after one yes in the chat, in Claude Code, Codex and any other client.
- This breaks the earlier rule that the yes never comes from the model: the server cannot tell the user's yes from an agent mistake or from text the agent read. The allowlist, the echo check, the rate limit and one attempt per plan still apply.
- `terminal` stays the default for new installs.
