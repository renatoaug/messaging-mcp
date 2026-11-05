# Telegram

Updated: 2026-11-05

> Telegram sends as your own user through MTProto, Telegram's protocol, with `grammers`. Not a bot. **The send path is tested against fakes and the request is checked offline; the login, a real send and a real search have not run yet.** Records: [ADR-004](../decisions/ADR-004-telegram-personal-mtproto.md), [ADR-006](../decisions/ADR-006-contact-search-suggests.md).

## Credentials and the session

1. Get an `api_id` and `api_hash` at [my.telegram.org](https://my.telegram.org) (API development tools). Creating that application is your step.
2. Put the hash in a 0600 file inside a 0700 folder, outside the repository. Set `telegram.apiId`, `telegram.apiHashFile` and `telegram.sessionFile` (a path that does not exist yet).
3. When you decide to create persistent access, run it yourself in a terminal:

   ```sh
   messaging-mcp telegram-login --config /private/path/messaging-mcp.config.json
   ```

   It explains what it creates, asks for `CREATE SESSION`, then for your phone number, the login code and your 2FA password, all hidden. It never creates a new account, never sends a message, never prints the session and never overwrites an existing file. It refuses bots, and refuses a different user when `expectedUserId` is already set.
4. Set `telegram.expectedUserId` to the ID it prints.

The session file is JSON: the home datacenter and the authorization keys, the secret material Telegram uses to recognize this device. Anyone with it can act as you: revoke it under Telegram → Settings → Devices if it leaks. Do not repeat logins blindly when one fails: Telegram limits them.

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

It lists the results, asks which one, shows what will be stored and asks for `ALLOW maria`. `messaging-mcp revoke maria` removes it; plans prepared for it stop being sendable.

## Still open

- [ ] Run `telegram-login`, a send to Saved Messages and a search against a real account.
- [ ] Tell Telegram's definite rejections (for example `PEER_ID_INVALID`) apart from uncertain errors, so they become `failed`.
- [ ] Datacenter migration during a send is not handled: a session whose home datacenter moved fails as uncertain.
