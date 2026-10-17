#!/usr/bin/env bash
# Spike S7: one-line Haiku recap of a session transcript. Runs in WSL (called via wsl.exe).
# Usage: recap.sh <transcript.jsonl>
t="$1"
# Condense: last 20 user/assistant text entries, each truncated.
ctx=$(jq -r 'select(.type=="user" or .type=="assistant")
  | (.message.content | if type=="string" then . else (map(select(.type=="text") | .text) | join(" ")) end) as $txt
  | select($txt != "") | "\(.type): \($txt[0:300])"' "$t" | tail -n 20)
printf '%s\n' "$ctx" | CC_SIDEBAR_INTERNAL=1 claude -p --model haiku --no-session-persistence \
  --tools "" --strict-mcp-config --setting-sources "" \
  --system-prompt 'You write status recaps for a developer dashboard. Input is a condensed Claude Code session transcript. Output exactly two parts separated by a line "---": first a single line under 90 characters saying what the session did or is doing (past tense, no preamble); then 2-3 short lines of detail. Plain text only.'
