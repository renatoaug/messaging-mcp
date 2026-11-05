#!/bin/sh
# Every check CI runs, in the same order. The pre-push hook calls it; run it yourself before a PR.
set -e
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo deny check
cargo machete
# Coverage floor for the offline suite. The Telegram client, the logins and the WhatsApp bridge
# process only run against a real account or a phone, so they are left out here and covered by
# the first live test (docs/design/06-testing.md).
cargo llvm-cov --locked --fail-under-lines 80 \
  --ignore-filename-regex 'src/(main\.rs|adapters/telegram_(client|login)\.rs|setup/(telegram_qr|bridge_process|whatsapp_flow)\.rs)'
cargo run --quiet --locked --example demo
