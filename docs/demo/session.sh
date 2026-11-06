#!/usr/bin/env bash
# A scripted assistant session for the README's demo GIF (docs/demo/demo.tape). The names and
# messages are made up; the steps are the ones messaging-mcp really takes.
set -u
dim=$'\e[2m'; bold=$'\e[1m'; green=$'\e[32m'; cyan=$'\e[36m'; reset=$'\e[0m'

say() { printf '%s\n' "$1"; sleep "${2:-0.35}"; }
tool() { printf '%s⏺ %s%s\n' "$cyan" "$1" "$reset"; sleep 0.8; }
ask() { printf '\n%s❯ %s' "$bold" "$reset"; read -r _; printf '\n'; }

ask
tool 'messaging-mcp · read_messages (whatsapp, "alice")'
say "Alice sent you 2 messages this morning:"
say "  • \"are we still on for lunch tomorrow?\""
say "  • ${dim}[audio 0:12]${reset} \"if you can, bring Friday's slides, I want to show Bob\"" 0.8
say ""
say "Want to reply? In your usual tone:"
say "  1. yes!! 12:30 at the usual place? I'll bring the slides"
say "  2. still on 👍 slides included"
say "  3. can we do 1pm? bringing the slides" 0.5
ask
tool 'messaging-mcp · prepare_message'
say "To: ${bold}Alice${reset} (WhatsApp)"
say "  \"yes!! 12:30 at the usual place? I'll bring the slides\""
say "Send it?" 0.3
ask
tool 'messaging-mcp · send_message'
say "${green}✓ Sent to Alice.${reset}" 6
