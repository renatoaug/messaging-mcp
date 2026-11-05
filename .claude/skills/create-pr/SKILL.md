---
name: create-pr
description: Creates pull requests following this repo's conventions — Conventional Commits title and What/Why/Acceptance criteria sections. Use whenever the user asks to open, create, or submit a PR, and only after they said yes to that PR.
---

# Create pull request

Follow these steps, in order.

## 0. Ask first

Never open, edit, push to or merge a PR without the owner's explicit yes for that PR. Show what changed and ask "open the PR?". A request to change code or commit is not that yes.

## 1. Preparation

- Make sure the work is committed on a branch of its own (create one with a `feat/`, `fix/`, `chore/`, etc. prefix if needed), branched off `main`.
- `scripts/check.sh` passes. Say so in the acceptance criteria only if you ran it.
- Push the branch before creating the PR. The PR base is `main`: `gh pr create --base main ...`.
- **Never** include in commits or in the PR: "Generated with Claude Code" footers, session links (`https://claude.ai/code/...`), `Claude-Session:` trailers, or `Co-Authored-By: Claude`. Technical content only.
- **Never** paste tokens, sessions, phone numbers, real recipients or message text into the PR, even from a test you ran.
- Everything in English.

## 2. Title

```
type(scope): lowercase subject
```

- Types: `feat`, `fix`, `refactor`, `chore`, `docs`, `perf`, `test`, `ci`, `build`, `style`, `revert`.
- Scopes, matching the code: `mcp`, `cli`, `service`, `store`, `config`, `telegram`, `whatsapp`, `docs`, `ci`. The `labels.yml` workflow adds `type:` and `scope:` labels from the title when a label with that name exists.
- The subject **must not start with an uppercase letter**. The title becomes the release note: a `feat` bumps the minor version, anything else the patch.

## 3. Body

Follow `.github/PULL_REQUEST_TEMPLATE.md`:

```markdown
### What?

<what the PR does — concrete changes, as a list when there are several>

### Why?

<the problem or goal that justifies the change>

### Acceptance criteria

- [x] <what you already validated — e.g. "`scripts/check.sh` passes on macOS">
- [ ] <what a person has to verify — e.g. "live mode: approve and send a plan to Saved Messages; it arrives once, with `**asterisks**` literal">
```

- **If there is an issue, the PR closes it**: `Closes #NN` on the first line of **What?**. When it solves only part, `Part of #NN` and what is missing.
- Write the body to a temporary file and run `gh pr create --base main --title "<title>" --body-file <file>`.

## 4. Acceptance criteria

Split it in two, in this order:

1. **What you validated**, ticked: commands you actually ran, with their result. Never tick a box you did not run.
2. **What only a person can confirm**, unticked: anything that needs a real account, a real terminal for `approve` or `allow`, or a real MCP client. Name the command, the mode and the expected result.

A change that touches sending, approval, identity, receipts or the allowlist always has at least one unticked live item, or says why none applies. A PR with nothing a person needs to check says so in one line: `No behavior change a person needs to check.`

## 5. After creating

- Report the PR link back to the user.
