# ADR-001: One Rust binary for the MCP server and the CLI

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/01-architecture.md](../design/01-architecture.md), [ADR-004](ADR-004-telegram-personal-mtproto.md)

## Context

The first version was TypeScript on Node 24: each MCP client started `node dist/cli.js mcp`, which needed Node installed, a build step, and the experimental `node:sqlite` module for the ledger. The binary holds the owner's account sessions, so a small, reviewable dependency tree and strict types around the state machine matter more than in an ordinary tool. A spike confirmed the three pieces exist in Rust: the official MCP SDK (`rmcp`), SQLite (`rusqlite` with the library bundled) and a Telegram MTProto client (`grammers`) whose sender pool can turn retries off.

## Decision

**Messaging MCP is one Rust binary (`messaging-mcp`), built with the latest stable toolchain, that is both the stdio MCP server (`messaging-mcp mcp`) and the CLI.** It supports macOS and Linux only. The WhatsApp bridge stays a separate Go program ([ADR-005](ADR-005-whatsapp-external-bridge.md)).

## Alternatives considered

- **Stay on Node 24:** works, but needs Node and a build on every machine, and the ledger sits on an experimental module.
- **Bun:** runs TypeScript without a build and has a stable SQLite, but whether the Telegram library worked on it was unproven, and it still needs a runtime installed.
- **Rust with an in-process WhatsApp client (`whatsapp-rust`):** would remove Go, but it is a 0.x crate, not updated since August 2026, handling the owner's account. Kept for later.

## Consequences

- Clients point at a single file; release binaries need nothing installed.
- The compiler enforces the state machine (`Status`, `Mode`, `Channel` are enums), `unsafe` is forbidden and `unwrap` denied outside tests.
- The tooling differs from TypeScript projects: `rustfmt`, `clippy`, `cargo deny`, `cargo machete`, `cargo llvm-cov` replace Biome, knip and `bun test`. Docs, records, skills, hooks and release conventions stay the same.
- `grammers` is 0.x and its raw `invoke` is outside its semver promise: an update of that crate needs the offline request tests and a live send to Saved Messages.
- Windows is out: the security model relies on POSIX permissions.
