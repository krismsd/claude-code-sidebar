#!/usr/bin/env bash
# Run a command in Windows PowerShell, in the repo root, with PATH refreshed from the registry
# (WSL inherits a stale PATH from when it started). Usage: scripts/win.sh 'npm run tauri build'
set -euo pipefail
root="$(wslpath -w "$(cd "$(dirname "$0")/.." && pwd)")"
exec powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "\$env:Path=[Environment]::GetEnvironmentVariable('Path','Machine')+';'+[Environment]::GetEnvironmentVariable('Path','User'); Set-Location '$root'; $*; exit \$LASTEXITCODE"
