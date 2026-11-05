# ADR-003: Approving, allowing and logging in happen only in a terminal

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [SECURITY.md](../../SECURITY.md), [ADR-006](ADR-006-contact-search-suggests.md)

## Context

An agent reads untrusted text all day: web pages, files, and, if it ever reads messages, other people's words. Any tool it can call, it can be talked into calling. The steps that grant power over the account (logging in, adding a recipient, authorizing a specific live message) must not be callable by the agent through MCP.

## Decision

**There is no MCP tool to log in, approve a plan or allow a recipient. `approve`, `allow` and `telegram-login` are CLI commands that refuse to run unless stdin and stdout are an interactive terminal, show exactly what they authorize, and require typing a confirmation that names it (`APPROVE msg_…`, `ALLOW maria`, `CREATE SESSION`).** Dry-run plans need no approval, because nothing leaves the machine.

## Alternatives considered

- **Ask through the MCP client's own confirmation prompt:** the client asks before calling a tool, but the user approves a tool call, not a reviewed message, and many users allow tools permanently. It stays as an extra layer, not the gate.
- **An approval tool with a code the user types back:** the code passes through the agent, which can relay or invent it.
- **A separate approval service under another OS user:** the strong version of this boundary. Not built yet; see the limits in SECURITY.md.

## Consequences

- An agent alone cannot send a live message, add a recipient or create a session through MCP.
- Each live message costs the owner a terminal step within the plan's lifetime (10 minutes by default).
- The check does not stop an agent that has a shell running as the same user: it can drive a pseudo-terminal or edit the ledger. That limit is stated, not hidden.
