#!/usr/bin/env bash
# Remove cc-sidebar's WSL side: take our hooks out of ~/.claude/settings.json (backed up first;
# everything else is kept) and delete claudes, cc-hook and cc-recap from ~/.local/bin.
# To roll back to an exact earlier file instead, copy a settings.json.cc-sidebar-backup-* over it.
#   scripts/uninstall.sh [--dry-run]
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
bin="$HOME/.local/bin"
dry=""; [ "${1:-}" = "--dry-run" ] && dry="--dry-run"

python3 "$repo/scripts/cc_hooks.py" uninstall $dry
for f in claudes cc-hook cc-recap cc-tasks cc-usage; do
  if [ -n "$dry" ]; then echo "would remove $bin/$f"; else rm -f "$bin/$f"; fi
done
