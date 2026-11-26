# ADR-014: Save a received photo or file when the owner asks

- **Status**: Accepted
- **Date**: 2026-11-26
- **Related**: extends [ADR-012](ADR-012-read-chats-when-the-owner-asks.md); [design/03-telegram.md](../design/03-telegram.md#saving-a-file), [design/04-whatsapp.md](../design/04-whatsapp.md#saving-a-file), [SECURITY.md](../../SECURITY.md)

## Context

After reading a chat, the owner asks for the photos or PDFs someone sent ("open the deck Alice sent"). `read_messages` showed only a label such as `[file: deck.pdf]`. Telegram keeps the files on its servers. WhatsApp keeps an encrypted copy on its media servers for a while, but only a message's keys (location, media key, hashes) can fetch and decrypt it, and the bridge kept only text.

## Decision

**A new MCP tool, `save_attachment`, downloads the photo, video or file of one message the owner already read in this session and writes it to `~/Downloads/messaging-mcp/` (0700, the file 0600, never replacing a file). `read_messages` marks each message that has one with an `attachment` ID. The WhatsApp bridge keeps each photo's, video's and document's keys with its text for the same 3 days, never the file, and downloads it only when asked (`GET /api/attachment`). Telegram reads the message again for a fresh file reference and downloads it through grammers.** Voice messages are transcribed instead ([ADR-013](ADR-013-transcribe-voice-messages-locally.md)); stickers are left out. Files over 50 MB are refused. The name comes from the sender, so it is reduced to a plain, visible file name.

## Alternatives considered

- **Download every file as it arrives:** works even after WhatsApp drops the copy, but keeps other people's files on disk unasked, and costs bandwidth for files nobody opens.
- **Return the file's contents in the tool answer:** no file on disk, but a PDF in base64 fills the agent's context and a photo cannot be shown that way.
- **Save to the state directory:** private, but the owner wants to open the file, and messaging-mcp's folders are hidden and refused for sending.

## Consequences

- The agent can open a photo or PDF someone sent and summarize it, when the owner asks.
- Files that arrived on WhatsApp before this change, or more than 3 days ago, cannot be saved; WhatsApp may also drop its copy sooner.
- A file's contents reach the model like message text: data that may try to instruct the agent ([SECURITY.md](../../SECURITY.md)).
- Saved files stay in `~/Downloads/messaging-mcp/` until the owner deletes them.
