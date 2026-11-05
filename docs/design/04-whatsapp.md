# WhatsApp

Updated: 2026-11-05

> WhatsApp goes through a separate Go program, the bridge from [verygoodplugins/whatsapp-mcp](https://github.com/verygoodplugins/whatsapp-mcp), built on `whatsmeow`. It links to your account as a WhatsApp Web device. You run one bridge; every client uses it. **The contract is tested against a local fake; the bridge itself has not been built or paired yet.** Record: [ADR-005](../decisions/ADR-005-whatsapp-external-bridge.md).

## The pinned contract

Checked at commit [`a1af56c4`](https://github.com/verygoodplugins/whatsapp-mcp/tree/a1af56c43a8e8ec4196f4c65e520a12d44bf79cc) on 2026-11-05 (the manifest says 0.7.0; compare by commit, not by tag). The bridge's endpoints are `/api/health`, `/api/send`, `/api/mark-read`, `/api/react`, `/api/download` and `/api/typing`. We use only:

```text
POST /api/send
Authorization: Bearer <token>
{"recipient": "<JID>", "message": "<text>"}

→ 200 {"success": true, "message": "..."}
```

| Answer | Our result |
|---|---|
| 200 with `"success": true` | `sent`, with no message ID (the bridge returns none) |
| 400, 401, 403, 404, 405 | `failed` (`WHATSAPP_REJECTED`): these reject before the send handler |
| Any other status, a redirect, a timeout, a body that is not JSON, `success` not `true`, over 16 KB | `outcome_unknown` |
| No token, or a token under 16 characters | `failed` before any request |

The bridge has no idempotency key, and its send keeps running even if our request times out. That is why a timeout is uncertain.

## Setting up the bridge

These are your steps, after reading the bridge's own README. Nothing here installs or starts it.

1. Build it with Go (the upstream release ships a binary for Linux x86-64 only).
2. Set these variables for the bridge process (they are the bridge's settings, not our config):

   ```sh
   WEBHOOK_ENABLED=false
   WHATSAPP_AUTO_DOWNLOAD_MEDIA=false
   FORWARD_SELF=false
   WHATSAPP_BRIDGE_PORT=8080
   ```

   Webhooks and media download are on by default upstream and are not needed to send.
3. Start one instance, from a stable private folder: it keeps `store/.bridge-token` (0600) and its databases there.
4. Pair it by scanning the QR code with your phone, when you decide to link the device.
5. Check which account is paired, then set `whatsapp.accountLabel` and `whatsapp.tokenFile` to `store/.bridge-token`.

Keep the bridge on loopback: no open port, tunnel or proxy. If you pair another account, generate a new token: our plans are bound to the token, so old ones stop being sendable.

## Allowing people

The bridge has no contact search, so WhatsApp recipients are added by exact number:

```sh
messaging-mcp allow whatsapp 5511977776666@s.whatsapp.net --id ana --label "Ana"
```

## Still open

- [ ] Build and pair the bridge, and send to your own number.
- [ ] `doctor` does not call `/api/health`; a check that the bridge is up and paired would help, without contacting anyone else.
- [ ] Replacing the bridge with an in-process client, which would remove Go ([ADR-001](../decisions/ADR-001-rust-single-binary.md)).
