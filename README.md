# cc-sidebar

A Windows sidebar (Tauri v2) that docks to a screen edge and shows every active Claude Code session as a card: project, status, live recap, alerts and actions.

- Design: [docs/SPEC.md](docs/SPEC.md)
- Milestones and spike findings: [docs/PLAN.md](docs/PLAN.md)

## Install

1. **Windows app:** run `src-tauri\target\release\bundle\nsis\cc-sidebar_<version>_x64-setup.exe` (per-user, no admin).
2. **Firewall (once, admin PowerShell):** hooks reach the app over the WSL network adapter.
   ```powershell
   New-NetFirewallRule -DisplayName "cc-sidebar: WSL hooks (TCP 47821)" -Direction Inbound -Action Allow -Protocol TCP -LocalPort 47821 -InterfaceAlias "vEthernet (WSL (Hyper-V firewall))" -RemoteAddress 172.16.0.0/12 -Profile Any
   ```
3. **WSL side:** `scripts/install.sh --dry-run` to preview, then `scripts/install.sh`. It installs `claudes`, `cc-hook` and `cc-recap` into `~/.local/bin` and merges the hooks into `~/.claude/settings.json` (backed up; other hooks and settings are kept).
4. Start sessions with `claudes` instead of `claude` for tab focus, PIDs and the `[cc:<tag>]` title. Plain `claude` sessions still show up.

Settings: tray icon → **Settings…** (`%APPDATA%\cc-sidebar\config.toml`, applied live).

## Turning it off

- **Instantly, without editing settings:** `mkdir -p ~/.config/cc-sidebar && touch ~/.config/cc-sidebar/off` (every hook exits immediately). Remove the file to turn it back on.
- **If the app isn't running:** hooks notice within 0.3 s and stop trying for 15 s at a time, so Claude Code isn't slowed down.
- **Uninstall:** `scripts/uninstall.sh` removes only our hooks and scripts. Each change leaves `settings.json.cc-sidebar-backup-<timestamp>` next to the file.

## Development (from WSL)

```bash
scripts/win.sh 'npm install'
scripts/win.sh 'npm run tauri build -- --debug --no-bundle'   # src-tauri/target/debug/cc-sidebar.exe
scripts/win.sh 'Set-Location src-tauri; cargo test --lib'
```

`scripts/win.sh` runs the command with the Windows toolchain (refreshed PATH, process-scoped execution policy bypass).

Test sessions load the hooks from the repo instead of global settings, and run only in `~/cc-sidebar-sandbox/`:

```bash
python3 dev/gen-hooks.py   # regenerates dev/hooks.json (adds a test-only deny rule)
spikes/s2-focus/tab-hooked.sh <repo> [mode] [name]   # inside a WT tab; drive with tmux -L cc-<name> send-keys
```
