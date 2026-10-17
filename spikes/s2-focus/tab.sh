#!/usr/bin/env bash
# Runs inside a test WT tab: set our own OSC title, then start claude with title updates disabled.
# Usage: tab.sh <tag> <sandbox-repo>
cd ~/cc-sidebar-sandbox/"$2" || exit 1
printf '\033]0;[cc:%s] %s\007' "$1" "$2"
CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1 exec claude
