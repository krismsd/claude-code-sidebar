#!/usr/bin/env bash
# Build and install a new cc-sidebar version end to end (from WSL):
#   scripts/release.sh [version]     e.g. scripts/release.sh 0.3.0
# 1. optionally sets the version (package.json, Cargo.toml, tauri.conf.json)
# 2. runs every check (lint, types, format, frontend + Rust tests, clippy); stops on failure
# 3. builds the installer, upgrades the installed app, updates the WSL scripts
#    (install.sh leaves ~/.claude/settings.json alone unless our hooks changed)
set -euo pipefail
cd "$(dirname "$0")/.."
win() { echo "+ $*"; scripts/win.sh "$*"; }

if [ -n "${1:-}" ]; then
  python3 - "$1" <<'PY'
import json, re, sys
v = sys.argv[1]
for p in ["package.json", "src-tauri/tauri.conf.json"]:
    c = json.load(open(p)); c["version"] = v
    open(p, "w").write(json.dumps(c, indent=2) + "\n")
p = "src-tauri/Cargo.toml"; s = open(p).read()
open(p, "w").write(re.sub(r'(?m)^version = ".*?"', f'version = "{v}"', s, count=1))
print(f"version {v}")
PY
fi

win 'npm run lint'
win 'npx tsc --noEmit'
win 'npm run format:check'
win 'npm test'
win 'Set-Location src-tauri; cargo fmt --check'
win 'Set-Location src-tauri; cargo clippy --quiet --all-targets -- -D warnings'
win 'Set-Location src-tauri; cargo test --lib --quiet'
win 'npm run tauri build'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w scripts/reinstall-app.ps1)"
scripts/install.sh
echo "released $(python3 -c 'import json; print(json.load(open("package.json"))["version"])')"
