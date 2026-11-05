---
name: add-channel
description: The pattern for adding a messaging channel (or changing one) in messaging-mcp — the adapter trait, the outcome rules, config, identity, tests and docs. Use before writing or changing code in src/adapters/.
---

# Add or change a channel

A channel is an adapter that turns one approved plan into one provider call and reports honestly what it knows. The two existing ones are the examples: `src/adapters/whatsapp.rs` (HTTP to an external program) and `src/adapters/telegram.rs` with `telegram_client.rs` (a library talking to the provider).

## The pieces, in order

1. **`Channel`** in `src/plan.rs`: add the variant, and its `as_str`/`parse`.
2. **Recipient** in `src/config.rs`: a variant of `Recipient` with an exact, validated `target` (an ID the provider cannot reassign, never a display name or a username). Add the check to `Recipient::validate` and the display to `display_target`.
3. **Config section** in `src/config.rs`: a struct with `deny_unknown_fields`, safe defaults, ranges checked in `parse_config`, secret paths made absolute.
4. **Identity** in `src/identity.rs`: what the plan sends as, and a fingerprint over every credential that decides the account, so rotating one invalidates approved plans. Read secrets with `secret_from(env, ...)`, never `std::env` directly.
5. **The adapter**: implement `ChannelAdapter::send` and return `Result<Receipt, DeliveryError>`.
6. **Wiring** in `src/app.rs` and `Adapters` in `src/service.rs`.

## The outcome rules (do not vary)

| What happened | `Outcome` |
|---|---|
| Nothing left the machine: missing credential, wrong account, invalid target | `NotSent` |
| The provider answered with a rejection documented to happen before sending | `NotSent` |
| Timeout, connection dropped after the request, unexpected answer, any doubt | `Unknown` |

- One attempt. No retry, no backoff, no reconnect-and-resend. Turn the library's own retries off.
- Error messages are ours. Never put a provider error's text, a URL, a token or the message in a `DeliveryError`.
- A message ID in the receipt only when the provider gave it for this exact request.
- Plain text only, unless a record says otherwise.
- Network code that cannot run offline goes in its own file, kept small; everything else stays testable.

## Tests (in the same change)

- In `tests/adapters.rs`: the exact request, success, each `NotSent` case sends nothing, each `Unknown` case is attempted exactly once, secrets never appear in errors.
- In `tests/identity.rs`: the fingerprint changes when a credential changes.
- In `tests/config.rs`: invalid targets and unknown fields are refused.
- `scripts/check.sh` passes, including the 80% line floor.

## Docs (in the same change)

- A record in `docs/decisions/` for how the channel reaches the provider and why.
- A design note `docs/design/NN-<channel>.md` with setup, the contract and its limits, and its row in `docs/design/README.md`.
- Update `README.md` and `SECURITY.md` where they list channels.

## Before saying done

- [ ] Dry-run calls no adapter for the new channel
- [ ] Every error path is `NotSent` or `Unknown` per the table, with a test each
- [ ] No provider text reaches an error
- [ ] Record, design note and indexes updated
- [ ] What still needs a real account is listed under "Still open"
