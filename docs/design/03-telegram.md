# Telegram

Updated: 2026-11-05

> Telegram sends as your own user through MTProto, Telegram's protocol, with `grammers`. Not a bot. **Tested end to end on 2026-11-05: the agent created the app, logged in by QR (twice, including a full reset), and sent two messages to Saved Messages after approval in Claude Code. Contact search has not run against a real account yet.** Records: [ADR-004](../decisions/ADR-004-telegram-personal-mtproto.md), [ADR-006](../decisions/ADR-006-contact-search-suggests.md), [ADR-007](../decisions/ADR-007-agent-runs-setup.md).

## Credentials and the session

1. **The app.** The assistant opens [my.telegram.org](https://my.telegram.org) in a browser it can drive. You sign in there (the code arrives in your Telegram app). It opens API development tools, creates an app ("Messaging MCP", platform Desktop), reveals the `api_hash`, calls `telegram_set_credentials` and hides the hash again. The hash goes to `telegram-api-hash` (0600) and is never returned. Telegram allows one app per account.
2. **The login.** `telegram_login` asks Telegram for a login token (`auth.exportLoginToken`), draws it as a QR code on a page in `~/Downloads`, and waits. You scan it in Telegram → Settings → Devices → Link Desktop Device. Tokens expire in about 30 seconds; the server asks for a new one and redraws the page, and notices the scan through Telegram's `updateLoginToken`. A token from another datacenter is imported there (`auth.importLoginToken`).
3. **A 2FA password**, if the account has one: the server saves the half-done session as `telegram-pending-session.json` (0600) and tells the agent to ask you to run `messaging-mcp telegram-password` in your terminal. You type the password hidden; it never passes through the chat.
4. **The result**: the session file and `telegram.expectedUserId` are written, and the page turns into "You're connected". A session saved without its user ID (a reconnect at the wrong moment) is adopted on the next `telegram_login`: the server asks Telegram whose session it is instead of asking for a new scan.

`messaging-mcp telegram-login` is the terminal alternative: phone number, code and password, all hidden. It never creates an account, never prints the session and never overwrites an existing file. Both logins refuse bots and refuse a different user than `expectedUserId`.

The session file is JSON: the home datacenter and the authorization keys, the secret material Telegram uses to recognize this device. Anyone with it can act as you: revoke it under Telegram → Settings → Devices if it leaks. To log in again, end the "Messaging MCP" device there, delete the session file and remove `expectedUserId`. Do not repeat logins blindly when one fails: Telegram limits them.

## Targets

| `kind` | Fields | What it is |
|---|---|---|
| `self` | — | Your Saved Messages |
| `user` | `id`, `accessHash` | A person |
| `chat` | `id` | A basic group |
| `channel` | `id`, `accessHash` | A supergroup or channel |

IDs are positive decimal numbers, without the Bot API's `-100` prefix. The access hash is a number Telegram gives your session for that peer; it cannot be guessed. Usernames are not accepted as targets, because a username can change owner. `allow telegram` fills all of this for you.

## How a send works

1. Read the credentials and session; any problem is `TELEGRAM_UNAVAILABLE`, not sent.
2. Ask Telegram whether the session is authorized, then who it belongs to. Not authorized is `TELEGRAM_NOT_AUTHORIZED`; a bot or another user is `TELEGRAM_ACCOUNT_MISMATCH`. Both are "not sent".
3. Call `messages.sendMessage` once: the plan's text exactly, no entities (so `**this**` stays literal), no link preview, no paid stars, no paid flood skip, and a `random_id` derived from the plan ID.
4. Close the connection. A close error never replaces the result.

From step 3 on, any error or a timeout is `TELEGRAM_OUTCOME_UNKNOWN`. That includes errors Telegram might have meant as "rejected": we do not tell them apart yet, so we assume the worst. The sender pool has `NoRetries`, and a flood wait is never slept through.

The receipt's message ID comes only from the update that carries our `random_id`, never from another update in the same answer.

## Finding and allowing people

`search_contacts` (MCP) and `messaging-mcp allow telegram QUERY --id ID` (CLI) call `contacts.search` and keep only `my_results`: your contacts and chats, never Telegram's global directory. They run only in live mode, take 2–64 characters and return up to 20 results.

They drop yourself, deleted accounts, bots, and peers without a full access hash. Names have control characters removed and are cut to 100 characters. The agent sees:

```json
{ "channel": "telegram", "kind": "user", "reference": "user:42", "name": "Maria Silva", "username": "maria", "recipientId": "maria" }
```

`recipientId` is present when that person is already allowed. To allow someone:

```sh
messaging-mcp allow telegram "Maria" --id maria
```

From the assistant, `allow_recipient` with the result's `reference` (`user:42`) does the same, only when you asked to message that person; each live message still needs your yes. In a terminal, `allow telegram` lists the results, asks which one, shows what will be stored and asks for `ALLOW maria`. `revoke_recipient` or `messaging-mcp revoke maria` removes it; plans prepared for it stop being sendable.

## Still open

- [x] A QR login and a send to Saved Messages against a real account → done on 2026-11-05.
- [ ] A contact search against a real account.
- [ ] `telegram-login` (phone code) and `telegram-password` against a real account.
- [ ] Tell Telegram's definite rejections (for example `PEER_ID_INVALID`) apart from uncertain errors, so they become `failed`.
- [ ] Datacenter migration during a send is not handled: a session whose home datacenter moved fails as uncertain.
