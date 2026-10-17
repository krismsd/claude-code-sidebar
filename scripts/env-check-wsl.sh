#!/usr/bin/env bash
# cc-sidebar environment check (WSL side). Read-only: prints info, changes nothing.
# Run inside WSL:  bash env-check-wsl.sh
h() { printf '\n=== %s ===\n' "$1"; }
v() { printf '%s: ' "$1"; shift; "$@" 2>/dev/null | head -n1 || echo 'NOT FOUND'; }

h 'Distro'
echo "WSL_DISTRO_NAME=$WSL_DISTRO_NAME"
grep -E '^(PRETTY_NAME)=' /etc/os-release
v 'networking mode' wslinfo --networking-mode
echo "default route (Windows host in NAT mode): $(ip route show default 2>/dev/null | awk '{print $3}')"

h 'Claude Code'
v 'claude path' command -v claude
v 'claude version' claude --version
v 'node' node --version
echo "ANTHROPIC_API_KEY set: $([ -n "$ANTHROPIC_API_KEY" ] && echo yes || echo no)"
echo "CLAUDE_CODE_USE_BEDROCK/VERTEX: ${CLAUDE_CODE_USE_BEDROCK:-unset}/${CLAUDE_CODE_USE_VERTEX:-unset}"

h 'Claude settings (keys only, no values)'
for f in ~/.claude/settings.json ~/.claude/settings.local.json; do
  if [ -f "$f" ]; then
    echo "$f"
    if command -v jq >/dev/null; then
      echo "  top-level keys: $(jq -r 'keys | join(", ")' "$f")"
      echo "  hook events: $(jq -r '(.hooks // {}) | keys | join(", ")' "$f")"
      echo "  defaultMode: $(jq -r '.permissions.defaultMode // "unset"' "$f")"
      echo "  env keys: $(jq -r '(.env // {}) | keys | join(", ")' "$f")"
    else
      echo '  (jq not installed - cannot summarise)'
    fi
  else
    echo "$f: not present"
  fi
done

h 'Where your projects live'
echo "Distinct cwd of running claude processes:"
for p in $(pgrep -x claude 2>/dev/null); do readlink "/proc/$p/cwd"; done | sort -u
echo "Project dirs Claude Code knows about: $(ls ~/.claude/projects 2>/dev/null | wc -l)"

h 'Tools'
for t in jq curl tmux python3 git wslpath; do v "$t" command -v "$t"; done

h 'Done'
echo 'Review before sharing - no secrets are printed, but paths may reveal project names.'
