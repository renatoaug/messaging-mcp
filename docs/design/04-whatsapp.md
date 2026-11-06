# WhatsApp

Updated: 2026-11-06

> WhatsApp goes through our own bridge, `messaging-mcp-whatsapp-bridge`, a small Go program on `whatsmeow` that links to your account as a WhatsApp Web device. One bridge runs on the machine; every client uses it. The assistant starts and pairs it with `whatsapp_setup`. **Tested end to end on 2026-11-05: paired by QR, one message sent to the owner's own number after approval in Claude Code.** Records: [ADR-008](../decisions/ADR-008-own-whatsapp-bridge.md), [ADR-007](../decisions/ADR-007-agent-runs-setup.md).

## The bridge

The source is in `whatsapp-bridge/` (`main.go`, `server.go`, `server_test.go`). It keeps the device session in `store/session.db` and the text of the last 3 days of messages in `store/messages.db` (0600, [reading a chat](#reading-a-chat)); no stored media, no webhooks, and a file is held in memory only while it is sent. It never logs message contents. It listens only on `127.0.0.1`, and every route needs `Authorization: Bearer <token>`; the token is created on first start in `store/.bridge-token` (0600). The SQLite driver is pure Go, so it builds without a C compiler; releases ship it next to `messaging-mcp`.

```text
GET  /api/health → 200 {"connected": true, "paired": true, "jid": "5511999999999:12@s.whatsapp.net"}  (503 when not connected)
GET  /api/qr     → 200 {"code": "2@…", "paired": false, "connected": false}
POST /api/send   {"recipient": "<JID>", "message": "<text>"} → 200 {"success": true, "id": "3EB0…"}
POST /api/send   {"recipient": "<JID>", "message": "<caption>", "file": {"name": "a.pdf", "mime": "application/pdf", "kind": "document", "data": "<base64>"}}
```

| `/api/send` answer | Our result |
|---|---|
| 200 with `"success": true` | `sent`, with WhatsApp's message ID |
| 400 (bad JSON, unknown field, not a phone or group JID, blank text without a file, an invalid file), 401 (token), 409 (not connected), 422 (the file upload to WhatsApp failed, before any message) | `failed` (`WHATSAPP_REJECTED`): nothing was sent |
| 502 (the send failed), any other status, a redirect, a timeout, an odd body | `outcome_unknown`: the message may have left |
| No token, or a token under 16 characters | `failed` before any request |

A file is `kind` `image` (sent as a photo) or `document` (keeps its name), at most 50 MB; text sends time out after 30 s, file sends after 5 minutes.

The bridge exits with code 4 when a pairing times out unscanned, and 5 when the device is unlinked from the phone (it then forgets the session, so the next start pairs again).

## Setting up

`whatsapp_setup` does it:

1. **Find the bridge** next to `messaging-mcp`. If it is missing, the error says how to install it from the release archive or build it: `cd whatsapp-bridge && go build -o ~/.local/bin/messaging-mcp-whatsapp-bridge .`
2. **Configure**: writes `whatsapp.baseUrl` (`http://127.0.0.1:8080/api`) and `whatsapp.tokenFile` (`~/.config/messaging-mcp/whatsapp-bridge/store/.bridge-token`).
3. **Start**: launches `messaging-mcp whatsapp-bridge-supervise` in its own process group, so the bridge keeps running when the MCP client closes. The supervisor runs the bridge with its output in `bridge.log` (owner-only), and every second asks `/api/qr` for the current code and redraws `messaging-mcp-whatsapp-pairing.html` in `~/Downloads`.
4. **Pair**: you scan the page in WhatsApp → Settings → Linked devices → Link a device. WhatsApp offers about 2.5 minutes of codes per attempt; the supervisor starts a timed-out pairing again up to 4 times, so you have about 10 minutes. The page turns into "You're connected" when it works, or "The code expired" after that.
5. **Confirm**: `whatsapp_status` waits up to 25 seconds for `/api/health` to report connected and paired, then sets `whatsapp.accountLabel` to the paired number the bridge reports.

`whatsapp_stop` (or `messaging-mcp whatsapp-bridge-stop`) stops the supervisor and the bridge; the pairing stays, so the next start reconnects without a QR code.

When an MCP client starts its `messaging-mcp mcp` process and the bridge was paired before (`whatsapp.accountLabel` is set and `store/session.db` exists) but is not running, for example after the machine restarted, the server starts it again, with no QR code. If a send finds the bridge down anyway, the connection is refused before any request, so the plan is `failed` with `WHATSAPP_BRIDGE_DOWN`, never uncertain. Only the supervisor that wrote `bridge.pid` removes it, so two clients starting at once do not lose track of the running one. If you pair another account, our plans stop being sendable, because they are bound to the bridge token.

## Allowing people

The bridge has no contact search, so WhatsApp recipients are added by exact number: the assistant with `allow_recipient` and a `whatsappJid`, or you in a terminal:

```sh
messaging-mcp allow whatsapp 5511977776666@s.whatsapp.net --id ana --label "Ana"
```

A JID is the country code and number, then `@s.whatsapp.net`. Some Brazilian accounts are registered without the extra 9 of mobile numbers, as the live test showed. When in doubt, use the number `whatsapp_status` reports for yourself.

## Reading a chat

WhatsApp keeps no history on its servers, and a linked device cannot reliably fetch old messages, so the bridge keeps what it sees ([ADR-012](../decisions/ADR-012-read-chats-when-the-owner-asks.md)): every message it receives or sends, as text (media as a label such as `[photo]` or `[file: a.pdf]` with its caption), in `store/messages.db` (0600). Reactions, edits and other protocol messages are skipped. A voice message is kept as `[audio 0:42] (not transcribed)`, then queued: one at a time, the bridge downloads it (up to 20 MB, within 4 minutes), writes it to `store/transcribe/` (0700, the file 0600), runs `messaging-mcp --config CONFIG transcribe FILE`, replaces the text with `[audio 0:42]` and what was said, and deletes the audio ([ADR-013](../decisions/ADR-013-transcribe-voice-messages-locally.md)). The supervisor passes `-transcriber` (its own path) and `-config` to the bridge; without them voice messages stay untranscribed. A chat reached through a hidden ID is stored under the person's number when WhatsApp knows it. Messages older than 3 days are deleted on start and on every new message.

```text
GET /api/chats?q=alice                      → 200 {"chats": [{"jid": "5511988887777@s.whatsapp.net", "name": "Alice"}]}
GET /api/messages?chat=<JID>&limit=20        → 200 {"messages": [{"from": "Alice", "fromMe": false, "at": 1793964129370, "text": "oi"}]}
```

`/api/chats` matches 2–64 characters, ignoring case and accents, against the account's contacts, its groups and the names in stored messages, up to 20. `/api/messages` returns the last 1–50 messages, oldest first. `read_messages` with `channel: whatsapp` calls them; a phone number is read directly.
