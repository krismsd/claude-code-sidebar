#!/usr/bin/env bash
# Install cc-sidebar's WSL side (SPEC §5): the claudes wrapper, cc-hook shim and cc-recap into
# ~/.local/bin, and cc-sidebar's hooks merged into ~/.claude/settings.json (backed up first;
# other hooks and settings are kept). Safe to re-run.
#   scripts/install.sh [--dry-run]
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
bin="$HOME/.local/bin"
dry=""; [ "${1:-}" = "--dry-run" ] && dry="--dry-run"

for f in claudes cc-hook cc-recap cc-tasks cc-usage; do
  if [ -n "$dry" ]; then echo "would install $bin/$f"; else install -D -m 755 "$repo/dev/bin/$f" "$bin/$f"; fi
done
command -v jq >/dev/null || echo "warning: jq not found (cc-recap needs it)" >&2
python3 "$repo/scripts/cc_hooks.py" install --shim "$bin/cc-hook" $dry
[ -n "$dry" ] || echo "Done. Start sessions with: claudes   (plain claude also works, with fewer features)"
