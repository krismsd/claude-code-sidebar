# cc-sidebar

A Windows desktop sidebar (Tauri v2) that docks to a screen edge and shows every active Claude Code session as a stacked card: project, status, live recap and alerts, with actions (focus tab, approve/deny, send prompt, resume, kill).

You (Claude) are the lead developer on this project. The user you're working with is the product owner and only user. Read `docs/SPEC.md` for the design and `docs/PLAN.md` for milestones before starting work. Keep both up to date as decisions change: they are the source of truth, not chat history.

## Environment

- The user runs Windows 11 with Windows Terminal. Claude Code (including this session) runs inside **WSL**.
- This repo lives on the Windows drive: `C:\dev\cc-sidebar` = `/mnt/c/dev/cc-sidebar`. That is deliberate: the Tauri app must be compiled by the Windows toolchain.
- WSL interop lets you run Windows programs from bash: `powershell.exe`, `cmd.exe`, `wt.exe`, `cargo.exe`, `npm.cmd`. Convert paths with `wslpath -w <path>` / `wslpath -u <path>`.
- Run Windows builds through PowerShell so they use the Windows toolchain, not the Linux one:
  ```bash
  powershell.exe -NoProfile -Command "Set-Location C:\dev\cc-sidebar; npm run tauri build"
  ```
  Never run `cargo`/`npm` for the app from Linux directly: you would get a Linux build and pollute `node_modules`/`target` with Linux binaries.
- `npm run tauri dev` is long-running: start it in the background and read its log; don't block on it.

## Seeing the screen

You can't see the user's desktop directly, but you can screenshot it and read the PNG:

```bash
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w scripts/screenshot.ps1)" -Out 'C:\dev\cc-sidebar\.shots\latest.png'
# then Read /mnt/c/dev/cc-sidebar/.shots/latest.png
```

Use this to verify UI work (docking, layout, alert flashes, tab focus) instead of asking the user. Ask the user only when something needs a human: a judgement call, a credential, or an approval.

## Safety rules (non-negotiable)

1. **Never modify `~/.claude/settings.json` (or any global Claude Code config) without the user's explicit approval in this conversation.** Every Claude Code session on the machine runs the global hooks; a broken one can freeze all of them, including the user's day-to-day work.
2. Since the global install (2026-10-17) every session runs the installed hooks (`~/.local/bin/cc-hook`). To change hook behaviour, test with `dev/bin/*` in sandbox sessions first, then re-run `scripts/install.sh`. `dev/hooks.json` adds the test-only deny rule; never install that globally.
3. Test sessions run only in sandbox folders under `~/cc-sidebar-sandbox/` (create throwaway git repos there). Never point a test session at one of the user's real projects.
4. Only send keystrokes, focus changes or kill signals to Terminal tabs and processes your own tests created. The user's real sessions are off limits, even now the hooks are installed globally (2026-10-17): they show up in the sidebar, but never act on them in tests.
5. Your own dev session must not depend on the sidebar running. If a hook you are testing could block (PermissionRequest), it must only ever be loaded into test sessions.
6. When provoking auto-mode denials for testing, use harmless commands or a custom test deny rule (see SPEC "Testing denials"), never real destructive or exfiltrating commands.
7. Record raw hook payloads you capture into `dev/fixtures/` (scrub anything personal) and use them in unit tests.

## Decisions already made (don't relitigate without reason)

- Tauri v2, Rust backend, React + TypeScript + Vite frontend.
- Docked as a Windows **AppBar** that reserves screen space (default: right edge of primary monitor, 320 px, configurable).
- Data feed: Claude Code **command hooks** running the `cc-hook` shim, which curls a server inside the app on the WSL host IP. WSL stays on NAT networking (Tailscale + VPNs): never switch it to mirrored.
- Cards show **Claude's last answer** as written (a question it ends on pinned below), not a summary. Haiku runs only to label background commands (job or service), to title untitled sessions, and with them refreshes usage.
- Alerts: flash an emoji badge when things go wrong, especially auto-mode denials (`PermissionDenied`).
- Actions: focus tab, approve/deny permission, resume, kill (send prompt deferred, see below).
- Projects are first class (tracked, stable order, drag to reorder); sessions stay in the list until removed (no stars; Remove moves one to the closed section and ends what it runs); details as an expanded copy of the card on hover (ignores the mouse); session actions in a right-click menu (rename, copy ID, remove, force end, delete); tasks from trackers via Claude (`cc-tasks`: connectors or allowed CLI commands, read-only). Window: docked or floating, collapsible to a strip, choice of monitor.
- The session wrapper is `claudes` (not `cc`: that's the C compiler).
- Sessions can be hosted in tmux (`session_mode`, SPEC §3.14) on a private socket (`tmux -L cc-sidebar`); the user's own tmux is never touched. Only send keys (`tmux send-keys`) to tmux sessions your own tests started.
- Send-prompt is **deferred**: it needs Claude Code Channels, which the user's org policy disables (S5). No keystroke-injection fallback. Revisit only if the org enables channels.

## Building and releasing

- `scripts/release.sh [version]` runs every check, builds the installer, upgrades the installed app (`scripts/reinstall-app.ps1`) and updates the WSL scripts (`scripts/install.sh`). Use it instead of piecemeal builds once a change is done.
- During development: `scripts/win.sh '<cmd>'` for Windows commands; `npm test` (Vitest) and `cargo test --lib` for tests. The installed app owns port 47821, so test UI changes by reinstalling, not with a debug build alongside it.
- The user's live data is in `%APPDATA%\cc-sidebar` (config.toml, projects.toml, sessions.json, projects.json, ui.json). Don't put test projects there; if you must, remove them after.

## Unverified assumptions

M1 verified most of the original list (see PLAN findings). Still open:

- Host IP stability through sleep/resume and VPN connect/disconnect.
- Monitor changes and real multi-monitor placement (DPI changes are verified).
- Alert sounds audible; toasts while a full-screen app covers the sidebar; light theme.

## Conventions

- Rust: `cargo fmt` and `cargo clippy -D warnings` clean. Frontend: TypeScript strict, ESLint plus Prettier.
- The session state machine is pure Rust with unit tests driven by `dev/fixtures/*.json`.
- Small commits with clear messages. Ask the user before creating any remote repo.
- The GitHub repo is public. Before any push, run the privacy scan kept in local memory (`public-repo-privacy-scan`) over the unpushed commits and their messages, and scrub hits by rewriting the unpushed history. Never list the scanned names in tracked files, this one included.
- Screenshots go in `.shots/` (gitignored).
