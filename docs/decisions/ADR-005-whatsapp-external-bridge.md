# ADR-005: WhatsApp through one external bridge the owner runs

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/04-whatsapp.md](../design/04-whatsapp.md)

## Context

WhatsApp has no official API for personal accounts; the Business/Cloud API is for companies. The working option is a client that links as a WhatsApp Web device. A mature one, `whatsmeow`, is a Go library, and an existing bridge built on it exposes a small authenticated HTTP API. WhatsApp allows a limited number of linked devices, so each MCP client must not create its own.

## Decision

**Messaging MCP talks to one WhatsApp bridge (`verygoodplugins/whatsapp-mcp`'s Go bridge, contract pinned to commit `a1af56c4`) over loopback HTTP with its Bearer token. The owner builds, starts, pairs and stops the bridge; Messaging MCP never starts it, installs it or copies its code.** One request per send, no redirects, no proxy, a bounded response body, and only HTTP 400/401/403/404/405 count as "not sent".

## Alternatives considered

- **WhatsApp Business/Cloud API:** official, but not a personal account.
- **`whatsapp-rust` inside our binary:** no Go and no second process, but a young 0.x crate handling the owner's account. Kept for later ([ADR-001](ADR-001-rust-single-binary.md)).
- **Starting the bridge from the MCP server:** convenient, but every client would race to run it, and a background process with the owner's account would start without them deciding to.

## Consequences

- Building the bridge needs Go; the upstream release ships only a Linux x86-64 binary.
- The bridge does not report which account is paired or a message ID: `accountLabel` is the owner's own check, and a receipt is "accepted" with no ID.
- The bridge has no contact search, so WhatsApp recipients are added by exact number (`allow whatsapp`).
- A change to the upstream contract breaks sends; compare against the pinned commit before updating.
