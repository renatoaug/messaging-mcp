# ADR-011: A file is checked and copied when the message is prepared

- **Status**: Accepted
- **Date**: 2026-11-05
- **Related**: [design/01-architecture.md](../design/01-architecture.md#files), [design/05-storage.md](../design/05-storage.md), [SECURITY.md](../../SECURITY.md)

## Context

The owner wants the agent to send files, up to 50 MB, any kind, with an optional caption, on both channels. A file is a new way to leak: an agent that misreads a request, or follows text hidden in a page, could attach a key, a session or a `.env` file and send it to a recipient the owner allowed. And a path is not content: the file could change between the review and the send.

## Decision

**`prepare_message` takes an optional `filePath`. The server resolves links, refuses any path inside a hidden folder or naming a hidden file (which covers `~/.ssh`, `~/.config`, `.env` and `.git`), refuses messaging-mcp's own folders (the state, the config and every secret file's folder), and refuses empty files and files over 50 MB. It then copies the file into `stateDir/files/<planId>` (0600) and records its name, size, type and SHA-256 in the plan and its fingerprint. The send reads that copy, refuses it if its SHA-256 changed, and removes it after the single attempt.** Any other file is allowed, as the owner chose. A JPEG or PNG up to 10 MB, recognized by its first bytes, goes as a photo; everything else as a document, never recompressed.

## Alternatives considered

- **Only `~/Downloads`, or folders the owner lists:** safer against a confused agent, but the owner preferred any file.
- **Read the file at send time:** what leaves could differ from what was reviewed.
- **Store copies by content hash:** two plans could share one copy, and removing it would need reference counting.

## Consequences

- What the owner reviews (name, size, SHA-256, path) is exactly what leaves.
- With `agent` approval, a confused or manipulated agent can still send any non-hidden file the owner can read to an allowed recipient. The allowlist and the review in the chat are the protection.
- Copies of plans that expire unsent stay in `stateDir/files/` until the next prepare sweeps them.
- A failed upload sends nothing (`TELEGRAM_UPLOAD_FAILED`, or HTTP 422 from the bridge); only an error after the message request is uncertain.
