# WhatsApp

Updated: 2026-11-05

> WhatsApp goes through a separate Go program, the bridge from [verygoodplugins/whatsapp-mcp](https://github.com/verygoodplugins/whatsapp-mcp), built on `whatsmeow`. It links to your account as a WhatsApp Web device. One bridge runs on the machine; every client uses it. The assistant builds, starts and pairs it with `whatsapp_setup`. **The contract and the QR reading are tested offline; the bridge has not been built or paired against a real phone yet.** Records: [ADR-005](../decisions/ADR-005-whatsapp-external-bridge.md), [ADR-007](../decisions/ADR-007-agent-runs-setup.md).

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

`whatsapp_setup` does it, and each call waits up to 25 seconds before answering, so the assistant calls it again until it is done:

1. **Build** (first time only, a few minutes): checks `git` and `go`, clones the repository into `~/.config/messaging-mcp/whatsapp-bridge/src`, checks out the pinned commit, and runs `go build` with CGO (the upstream release ships a binary for Linux x86-64 only). The output goes to `build.log`.
2. **Configure**: writes `whatsapp.baseUrl` (`http://127.0.0.1:8080/api`) and `whatsapp.tokenFile` (`run/store/.bridge-token`) to the config.
3. **Start**: launches `messaging-mcp whatsapp-bridge-supervise` in its own process group, so the bridge keeps running when the MCP client closes. It runs the bridge from `run/` with `WEBHOOK_ENABLED=false`, `WHATSAPP_AUTO_DOWNLOAD_MEDIA=false`, `FORWARD_SELF=false` and `WHATSAPP_DEVICE_NAME="Messaging MCP"`. Webhooks and media download are on by default upstream and are not needed to send.
4. **Pair**: the bridge draws its QR code only on a terminal, so the supervisor gives it a pseudo-terminal, copies what it prints to `bridge.log` (owner-only, started over past 1 MB), reads each new code drawn in half-block characters and redraws it on `messaging-mcp-whatsapp-pairing.html` in `~/Downloads`. You scan it in WhatsApp → Settings → Linked devices → Link a device.
5. **Confirm**: `whatsapp_status` asks the bridge's `/api/health` with its token. Once it reports connected and paired, the server reads the paired number from the bridge's own device table (`store/whatsapp.db`, read-only), sets `whatsapp.accountLabel` to it, and the page turns into "You're connected".

`whatsapp_stop` (or `messaging-mcp whatsapp-bridge-stop`) stops the supervisor and the bridge; the pairing stays, so the next start reconnects without a QR code. The bridge listens only on `127.0.0.1`. If you pair another account, our plans stop being sendable, because they are bound to the bridge token.

## Allowing people

The bridge has no contact search, so WhatsApp recipients are added by exact number: the assistant with `allow_recipient` and a `whatsappJid`, or you in a terminal:

```sh
messaging-mcp allow whatsapp 5511977776666@s.whatsapp.net --id ana --label "Ana"
```

## Still open

- [ ] Build and pair the bridge against a real phone, and send to your own number.
- [ ] Restarting the bridge after the machine restarts: nothing does yet; call `whatsapp_setup` again.
- [ ] Replacing the bridge with an in-process client, which would remove Go ([ADR-001](../decisions/ADR-001-rust-single-binary.md)).
