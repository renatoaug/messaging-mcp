# ADR-007: The agent runs the setup; the owner approves each live message in their client

- **Status**: Accepted; the bridge part is superseded by [ADR-008](ADR-008-own-whatsapp-bridge.md): `whatsapp_setup` starts our installed bridge and builds nothing
- **Date**: 2026-11-05
- **Related**: supersedes [ADR-003](ADR-003-human-steps-outside-mcp.md), partly supersedes [ADR-005](ADR-005-whatsapp-external-bridge.md) and [ADR-006](ADR-006-contact-search-suggests.md); [design/02-setup.md](../design/02-setup.md), [SECURITY.md](../../SECURITY.md)

## Context

[ADR-003](ADR-003-human-steps-outside-mcp.md) kept logging in, allowing recipients and approving messages in a terminal. In use, that made the project something the owner operated by hand: create an app, type commands, copy IDs, open a second terminal for every message. The owner wants Claude Code or Codex to do the work and to be asked only where a person is unavoidable. Two things stay unavoidable: a phone scan to link a new device (Telegram and WhatsApp both require it), and a person's yes before a message leaves. The first live test, on 2026-11-05, showed that Claude Code does not offer MCP's approval dialog (elicitation), so the yes has to come through something the client does support.

## Decision

**The agent runs the setup through MCP tools: `setup_status`, `telegram_set_credentials`, `telegram_login` (QR), `whatsapp_setup`/`whatsapp_status`/`whatsapp_stop` (build, start and pair the bridge), `set_mode`, `allow_recipient` and `revoke_recipient`. The owner only scans QR codes, reads the my.telegram.org code to the agent, and types a Telegram 2FA password in a terminal (`messaging-mcp telegram-password`), never in chat.**

**A live message is sent only after the owner approves exactly what leaves. `send_message` must repeat the plan's recipient and whole text, and the server refuses a mismatch. The approval comes from, in order: the client's approval dialog (MCP elicitation) when the client offers one; otherwise, with `"approval": "client-prompt"`, the client's permission prompt for that call, which shows those arguments and must be kept on with an "ask" rule; otherwise `messaging-mcp approve` in a terminal (`"approval": "terminal"`, the default).** In every case the yes comes from the client or the terminal, never from the model.

## Alternatives considered

- **Keep [ADR-003](ADR-003-human-steps-outside-mcp.md):** the strongest boundary against a confused agent, but every message costs a terminal round trip. Rejected by the owner after using it.
- **Only MCP elicitation:** the cleanest yes, and implemented, but Claude Code does not offer it today, so live sends would be impossible there.
- **The agent asks in chat and relays the answer:** nothing tells the server whether the yes came from the person or from the model. Rejected.
- **No approval at all:** a message an agent reads could make it write as the owner to anyone allowed. Rejected.

## Consequences

- Setup is a conversation: one browser task, two scans, no commands, no copying IDs. It works without a browser too, but then the owner has to enter the Telegram credentials (not built yet; see the design note).
- The `client-prompt` mode is only as strong as the client's prompt. An "always allow" on `send_message`, or a client mode that approves tools without asking, removes the owner's yes. The setup docs tell the owner to add the "ask" rule and never to always-allow.
- An agent can now allow recipients and start the bridge. A live message still needs the owner's yes, which shows the exact destination.
- The MCP server writes the config: every write is validated and atomic, and the config folder must be owner-only.
- The terminal remains the fallback, and the CLI keeps `approve`, `allow` and `telegram-login` for people who prefer it.
