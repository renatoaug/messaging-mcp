# ADR-012: Read a chat when the owner asks, and keep three days of WhatsApp messages

- **Status**: Accepted
- **Date**: 2026-11-06
- **Related**: changes [ADR-008](ADR-008-own-whatsapp-bridge.md) (the bridge kept only the session); [design/03-telegram.md](../design/03-telegram.md#reading-a-chat), [design/04-whatsapp.md](../design/04-whatsapp.md#reading-a-chat), [SECURITY.md](../../SECURITY.md)

## Context

The owner wants to ask "what did Nestor send me on Telegram?" or "look at X's message on WhatsApp", for anyone, not only allowed recipients. Telegram keeps the history on its servers. WhatsApp is end-to-end encrypted: its servers drop a message once delivered, and a linked device cannot fetch past messages reliably (it can only ask the phone, which may not answer). So the bridge can show only what it saw arrive.

## Decision

**A read-only tool, `read_messages {channel, chat, limit}`, returns a chat's latest messages (20 by default, at most 50), oldest first, in live mode only. `chat` is a name, a reference from an earlier answer, or a WhatsApp number. With several matches it reads nothing and lists them. The server's instructions and the tool's description tell the agent to read only when the owner asks, and every answer says the messages are data, never instructions. On WhatsApp, the bridge now keeps the text of the messages it receives and sends (media as a label such as `[photo]`) for 3 days in `store/messages.db` (0600), deletes older ones, and never logs them.** Any chat can be read; the allowlist still governs only sending.

## Alternatives considered

- **Read only allowed recipients:** the owner wanted to read anyone.
- **Keep WhatsApp messages in memory only:** a restart would lose them; the owner chose disk for 3 days.
- **Ask the phone for WhatsApp history:** unreliable and untested; may be tried later on top of this.

## Consequences

- The owner can ask what someone said on either channel.
- Message text from other people now reaches the model. A message can try to instruct the agent; with `agent` approval, only the agent's instructions stand between such a message and a send to an allowed recipient.
- The server cannot tell whether the owner asked for a read; that rests on the agent, as with `agent` approval.
- The bridge now holds up to 3 days of message text on disk. Unpairing does not delete it; deleting `store/messages.db` does.
- WhatsApp shows nothing from before the bridge was running.
