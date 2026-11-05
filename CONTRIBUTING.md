# Contributing

Thanks for wanting to help Messaging MCP. This page is the short version: how a contribution starts, what it needs to pass and how a pull request is written.

## Where to start

| You have | Start with |
|---|---|
| A bug you can reproduce | An issue from the Bug report template |
| An idea or a new feature | An issue from the Feature request template, before writing code. Its **Safety** section is required: say what an agent could do with the feature that the owner did not want |
| A small, obvious fix (a typo, a doc, a one-line bug) | A pull request straight away |
| A security problem | Never an issue. See [SECURITY.md](SECURITY.md) |

## Running it

[`docs/design/02-setup.md`](docs/design/02-setup.md) builds the binary and describes every setting. You never need a real account to work on the code: dry-run and the test fakes cover the whole flow.

## Rules and checks

[`AGENTS.md`](AGENTS.md) has the code and safety rules. Read it before the first change.

Before a pull request:

1. `scripts/check.sh` passes. Turn on the pre-push hook once with `git config core.hooksPath .githooks` and it runs on every push.
2. A change to the state machine, the identity binding, the receipts or the allowlist carries regression tests.
3. A change to what is stored adds a migration and updates [`docs/design/05-storage.md`](docs/design/05-storage.md).

## Pull requests

- One change per pull request, branched off `main`.
- The title follows [Conventional Commits](https://www.conventionalcommits.org) with a lowercase subject (`fix(whatsapp): ...`). It becomes the release note: a `feat` bumps the minor version, anything else the patch.
- The body follows the template: **What?**, **Why?** and **Acceptance criteria**. Tick what you ran; leave unticked what needs a real account or a person at a terminal, naming the command and the state.

## License

By opening a pull request you agree that your contribution is licensed under the [MIT License](LICENSE).
