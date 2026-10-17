#!/usr/bin/env bash
# Runs inside a test WT tab: start a hooked, tagged session via the claudes wrapper, inside a tmux
# server of its own (socket cc-<name>) so tests can drive it with
#   tmux -L cc-<name> send-keys -t <name> ...
# instead of SendKeys (which is unreliable after UIA tab selection).
# Closing the WT tab only detaches tmux: claude keeps running. Clean up with
#   tmux -L cc-<name> kill-server
# (and note that a tab close then does NOT produce SessionEnd the way a real tab does).
# Usage: tab-hooked.sh <sandbox-repo> [permission-mode] [name]
repo="$1" mode="${2:-auto}" name="${3:-$1}"
cd ~/cc-sidebar-sandbox/"$repo" || exit 1
exec tmux -L "cc-$name" -f /mnt/c/dev/cc-sidebar/spikes/s2-focus/tmux.conf new-session -s "$name" \
  "CC_SIDEBAR_SETTINGS=/mnt/c/dev/cc-sidebar/dev/hooks.json /mnt/c/dev/cc-sidebar/dev/bin/claudes --permission-mode $mode"
