# cc-sidebar: Build plan

Tick items off as you go and add findings inline. Each spike ends with a short "Findings" note and any SPEC updates.

## M0: Environment and scaffold
- [x] Run `scripts/env-check-wsl.sh` and `scripts/env-check-windows.ps1` (via `powershell.exe -File`). Summarise the results here.
  - **Findings (2026-10-17):** WSL Ubuntu 24.04, WSL 3.0.3, networking **NAT**, no `.wslconfig`. Claude Code 2.1.302, claude.ai OAuth login (no API key) → Haiku runner option (a) `claude -p`. Existing global hooks: `Notification` only. jq/curl/git/python3 present in WSL.
  - Windows 11 Pro 26200, WT 1.24, WebView2 155, winget present. Single monitor 2752×1152 @ 125% (DPI 120), taskbar at bottom.
  - **Missing on Windows:** Rust, Node, Git. Visual Studio Community 2026 (v18) is installed but **without** the C++ workload (VC.Tools.x86.x64).
  - Probe: a Windows listener on `localhost:47821` is unreachable from WSL. A listener on the NAT host IP 172.25.192.1 is reachable (see the networking item below).
- [ ] List missing Windows prerequisites (Rust/rustup MSVC toolchain, Node LTS, VS Build Tools "Desktop development with C++", WebView2) and give the user exact install commands (`winget` preferred). Wait for confirmation.
- [x] Check the WSL networking mode. NAT. **Decision: stay on NAT** (Tailscale + VPNs). Hooks use the `cc-hook` command shim → Windows host IP (SPEC §3.2). Probe: host-IP listener reachable from WSL, no firewall rule needed.
- [x] `git init`. Scaffold Tauri v2 + React + TS + Vite in this folder. Template from create-tauri-app (react-ts), ESLint 9 + Prettier, placeholder sidebar UI. Release build 4 min (LTO), debug ~1.5 min clean / 7 s incremental. Screenshot `.shots/m0-hello.png`.
  - Run Windows commands with `scripts/win.sh '<cmd>'`: it refreshes PATH from the registry (WSL inherits a stale PATH) and uses `-ExecutionPolicy Bypass` for that process only (npm.ps1 and our .ps1 scripts are otherwise blocked).
  - Rust 1.99.1 (stable-msvc), Node 24.20, npm 11.19; Tauri 2.12, windows crate 0.62.
- [x] Create `~/cc-sidebar-sandbox/` with 2–3 throwaway git repos for test sessions. (`alpha`, `bravo`, `charlie`)

## M1: Spikes (de-risk before building)
Each spike can be a throwaway binary or script under `spikes/`.
- [x] **S1 hook transport:** a minimal Rust/axum (or even PowerShell) listener on `<wsl-host-ip>:47821` that logs bodies and headers, plus the `cc-hook` shim. Start a sandbox test session with `--settings dev/hooks.json` and confirm every event arrives with tag/PID/WT_SESSION. Measure shim overhead, and check whether the Tauri exe triggers a firewall prompt.
  - **Findings:** works. `dev/bin/cc-hook` → `spikes/hook-listener` on 172.25.192.1:47821: 10–20 ms per event including process spawn. Tag, claude PID and WT_SESSION all arrive as headers. Seen: SessionStart (startup, compact), SessionEnd, UserPromptSubmit, Pre/PostToolUse, PostToolUseFailure, CwdChanged, SubagentStart/Stop, Stop, PreCompact, Notification (permission_prompt), PermissionRequest, PermissionDenied. Not seen: StopFailure (needs a real API error), Notification idle_prompt (didn't fire in a tmux session idle for 4+ min).
  - Firewall: a new exe that listens on the WSL adapter triggers the "allow public and private networks" prompt. It was allowed for `hook-listener.exe`. The real app needs a port-scoped rule (TCP 47821, WSL interface only) created by the installer, so neither debug nor release builds prompt.
  - Gotchas for the state machine: `SubagentStop` also fires for Claude Code's internal helper agents (`agent_type: ""`, e.g. prompt suggestions), so ignore those. A background subagent's hand-back arrives as `UserPromptSubmit` with a prompt starting `<agent-message`, so don't show it as the user's last prompt. `Stop` can fire while `background_tasks` still lists a running subagent ("idle, background running"). `Stop` carries `last_assistant_message`.
  - Test sessions run in detached tmux (`tmux new -d … claude --settings dev/hooks.json`) and are driven with `tmux send-keys`: no windows or focus involved. Your global `Notification` hook also runs in them (harmless). `--bare` would isolate fully but needs an API key. Save the payloads to `dev/fixtures/`.
- [x] **S2 tab focus:** launch two WT tabs titled `[cc:test1]` and `[cc:test2]` running `claude` with `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`. Confirm the title persists. Use UI Automation (Rust `uiautomation` crate, or PowerShell first) to list WT tabs, select one, and bring the window forward. Prove it with screenshots, and test while another app has focus.
  - **Findings:** works (`spikes/s2-focus/wt-tabs.ps1`). OSC 0 title `[cc:<tag>] <project>` survives Claude Code's TUI through idle, working and done when `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` is set. UIA lists every WT window/tab (class `CASCADIA_HOSTING_WINDOW_CLASS`, TabItem + SelectionItemPattern) in 130–590 ms. Select + Alt-key trick + SetForegroundWindow brings the window forward from another app's focus. Restoring a minimised window is not yet tested.
  - `wt.exe` splits its command line on `;` even inside quotes. Launch tabs as `wsl.exe -- bash -l <script> args`, never inline shell with `;`. A login shell (`-l`) is needed for `~/.local/bin` (claude) to be on PATH.
  - Closing a multi-tab WT window shows a "close all tabs?" dialog; `close-window.ps1` confirms it in that window only.
- [x] **S3 permission hold:** a PermissionRequest HTTP hook held by the listener. Test allow, deny, no-decision release and timeout. Document exactly what the terminal shows in each case.
  - **Findings:** the terminal's permission prompt appears **immediately and in parallel** with the held hook; it does not wait for the hook. Whichever answers first wins:
    - hook `allow`: the command runs and the terminal prompt never shows (instant answer), or closes if already open (late answer at 10 s).
    - hook `deny` + message: shows "Denied by PermissionRequest hook"; Claude sees the message. **No** `PermissionDenied` event (that event is auto-mode only).
    - hook `{}` (no decision): the terminal prompt just stays.
    - user answers in the terminal first: Claude Code kills the hook process immediately, so the server sees the connection drop and the late answer goes nowhere. That drop is the signal to clear the card.
    - shim curl limit (75 s) hit: same as `{}`, the prompt stays and nothing runs.
    - `Notification` `permission_prompt` arrives ~6–11 s after the request.
  - Redesign (SPEC §3.6): no hold window and no "answer in terminal" button. Hold the request for as long as the prompt is open (hook timeout near the 600 s max), and clear the card on answer or disconnect.
- [x] **S4 denials:** provoke an auto-mode denial safely (SPEC §4). Capture the `PermissionDenied` payload and check for a reason field.
  - **Findings:** works. The `autoMode.soft_deny` prose rule in `dev/hooks.json` (marker `CC_SIDEBAR_DENY_TEST`) made the classifier deny `echo CC_SIDEBAR_DENY_TEST` on the first try. The payload has `tool_name`, `tool_input`, `tool_use_id` and `reason: "[Auto-Mode Bypass]"` (the classifier's category, not our rule text). A plain `permissions.deny` match does not emit `PermissionDenied` (per docs).
- [ ] **S5 channel:** ⚠ **blocked; send-prompt deferred** (2026-10-17) a minimal custom channel that pushes a hard-coded message into a sandbox session. Confirm PID/tag correlation and how the message appears to Claude.
  - **Findings:** correlation works: the channel server's `process.ppid` equals the claude PID (= `CC_SIDEBAR_PID` in hook headers), and it inherits `CC_SIDEBAR_TAG`. But **pushes are dropped**: Claude Code says "--dangerously-load-development-channels blocked by org policy … Have an administrator set channelsEnabled: true in managed settings". the user's login is a Claude Team org. Also shown: "server:cc-sidebar · no MCP server configured with that name" (the server was loaded via `--mcp-config`; it may need to be in `.mcp.json`, to recheck once the policy is enabled). **Needs the user's decision.**
- [x] **S6 AppBar:** a Tauri window registered as an AppBar on the right edge. Confirm maximised windows respect it, and that it survives DPI and monitor changes.
  - **Findings:** built straight into the app (`src-tauri/src/appbar.rs`): ABM_NEW/QUERYPOS/SETPOS, subclassed window proc for ABN_POSCHANGED, ABN_FULLSCREENAPP (drops topmost), WM_DISPLAYCHANGE/WM_DPICHANGED/WM_SETTINGCHANGE re-dock, ABM_REMOVE on destroy. A maximised window stops exactly at the sidebar; 320 DIP = 400 px at 125%. Space is released on exit. DPI change verified (`scripts/dpi.ps1`, same undocumented DisplayConfig call the Settings app uses): 125%→150% re-docked from 400 to 480 px and adjusted to the taller taskbar, and back to 125% restored 400 px exactly. **Not tested:** monitor add/remove (single monitor).
  - Note: the screen is 3440×1440 physical; DPI-unaware tools (our PowerShell scripts) see 2752×1152.
- [x] **S7 Haiku runner:** time a `claude -p --model haiku` call from Windows through `wsl.exe`. Confirm our hooks ignore it (`X-CC-Internal`) and that it doesn't leave session clutter.
  - **Findings:** `spikes/s7-haiku/recap.sh` via `wsl.exe -- bash -l …`: **5.7–6.3 s** end to end, with a good one-line recap plus detail. Required flags: `--system-prompt …`, `--tools ""` (without it Haiku treated the input as a chat and ran `ls /tmp`), `--strict-mcp-config` (else it loads all claude.ai connectors), `--setting-sources ""` (loads no user settings, so no hooks and no recursion), `--no-session-persistence` (no transcript left; verified). With hooks loaded, every event carries `X-CC-Internal: 1`.
- [ ] Update SPEC: remove the [verify] tags you've confirmed, and redesign anything that failed. **Check in with the user** with a short findings summary before M2.

## M2: Core
- [x] Hook server, session store and state machine, with unit tests from the fixtures. (`server.rs`, `store.rs`, `session.rs`, `events.rs`; 23 tests.) The server holds PermissionRequest (575 s) so a terminal answer shows up as a dropped connection; with no tool result 3 s later the card goes to needs_input (no hook event fires after a terminal "No"). Unknown events never create cards.
- [x] Project identity (git root, worktree, branch, colour) and projects.toml overrides. (`project.rs`.) Keyed off the payload `cwd`, which stays the project dir; Claude's `cd` only shows up in `CwdChanged` (`shell_cwd`). Branch re-read on Stop/SessionStart.
- [x] Liveness checks and stale/removed handling. (`liveness.rs`.) Use `wsl.exe -e`, never `wsl.exe --` (the default shell re-parses the joined command line and quoting is lost), and `[ -d /proc/$p ]`, not `kill -0` (EPERM false positives). Ended cards linger 10 s, dead cards 30 min.
- [x] `claudes` wrapper script (first named `cc`, renamed at install: `cc` is the C compiler). (`dev/bin/claudes`; `CC_SIDEBAR_SETTINGS` adds `--settings` for test sessions.)
- E2E verified with sandbox sessions: permission prompt, terminal "No" → needs_input, Stop recap, worktree project + branch, SessionEnd removal, kill -9 → dead (`.shots/m2-*`). Firewall: the port-scoped rule "cc-sidebar: WSL hooks (TCP 47821)".

## M3: Sidebar MVP
- [x] AppBar, the card list with ordering rules, status chips and timers, instant recap. (`src/components/SessionCard.tsx`, `src/App.tsx`.) Sessions sharing a project group under a thin header; order freezes while the mouse is over the sidebar; hover shows last prompt, alerts, model/mode.
- [x] Closed sessions (SPEC §3.3): SessionEnd/dead → `closed` card instead of removal, dismiss, 7-day prune, persisted to `sessions.json`. (Tab close sends SessionEnd reason `other`, verified 2026-10-17.)
- [x] Click to focus, and resume in a new tab if the tab is gone. (`focus.rs`, `uiautomation` crate.) Open session → focus its `[cc:<tag>]` tab (plain `claude`: project name); closed session → `wt.exe -w 0 new-tab … wsl.exe --cd <cwd> -e bash -lc "<launcher> --resume <id>"`. An open session whose tab isn't found is never resumed (it would duplicate it); a toast says so. Resume keeps the same session_id, so the card revives in place.
- [x] Tray icon, config file, light/dark theme. Tray: Settings… (opens config.toml) and Quit; default Tauri icon until M7. `%APPDATA%\cc-sidebar\config.toml` is created on first run and hot-reloaded (edge left/right and width re-dock live; thresholds; launcher + launch_env). Dark theme verified; light theme not yet checked on screen.
- [x] Demo: screenshot plus a 3–4 sandbox session run. Four sandbox sessions in WT tabs: permission card, ⏳ waiting badge, 🚫 denial badge, worktree + branch, project grouping, click-to-focus, /exit → closed → click → resumed, app restart restores all cards (`.shots/m3-*`).

## M4: Alerts
- [x] 🚫 ⚠️ 🔁 ⏳ 🐌 📦 badges with flash animation, dismiss, and optional sounds and toasts.
  - New alerts and new permission prompts flash their card for 4 s (red / amber ring) and pop their badge; whatever exists at startup doesn't flash or sound. The newest alert shows as a line on the card with ✕; badges dismiss on click. `prefers-reduced-motion` gets a static ring.
  - `alerts.rs`: one sound per batch from `[sounds]` (system alias or .wav; defaults on for 🚫 ⚠️ 🔁 ⏳, off for 🐌 📦 and permission). Toasts: `toasts = "when_hidden"` (a full-screen app covers the sidebar, from ABN_FULLSCREENAPP), "always" or "never". Toast verified with "always" (dev build shows it as Windows PowerShell; the installed app will have its own identity). Not verified: hearing the sounds, and the when_hidden path with a real full-screen app.
  - Global hotkey `jump_hotkey = "Ctrl+Alt+J"` focuses the session waiting on you longest; verified from another app's focus. Re-registered on config reload.
  - Denial `reason` varies: a tag (`[Auto-Mode Bypass]`) or a sentence ("The server-side auto mode classifier judged this action dangerous…"). Alert text is now "Blocked: <what> (<reason>)".
  - Test tabs now run each session in its own tmux server (`tab-hooked.sh <repo> <mode> <name>`, drive with `tmux -L cc-<name> send-keys`): SendKeys after UIA tab selection proved unreliable.

## M5: Haiku recaps
- [x] Recap worker, debouncing, detail-on-hover.
  - `recap.rs` + `dev/bin/cc-recap`: on Stop, wait 3 s, then `wsl.exe -e bash -lc 'cc-recap "$1"' _ <transcript>`; one task per session, ≥30 s apart (`recap_min_interval_s`), max 2 in parallel. Results carry the Stop's `stop_seq`, so a late result for an older turn is dropped (and the newer turn recapped).
  - The card shows the instant recap in italics right after Stop and the Haiku line once it lands (4–6 s measured); hover shows 2–3 detail lines.
  - Prompt tuning: the condensed input omits tool results, so the prompt tells Haiku not to judge verification; file tools are passed as basenames (absolute paths confused it about "the working directory").
  - Each Stop costs one Haiku call on the subscription. `recap_command = ""` turns them off.

## M6: Actions
- [x] Approve/deny from the card (PermissionRequest hold). Allow/Deny buttons appear only while the request is actually held (`pending_permission.answerable`); the answer goes down a per-session oneshot channel. Verified: Allow runs the command and closes the terminal prompt; Deny shows "Denied by PermissionRequest hook" and Claude carries on.
- [ ] ~~Send prompt via `cc-channel`.~~ Deferred (2026-10-17): channels are disabled by the org policy. Revisit if the org enables `channelsEnabled`.
- [x] Kill, and open folder or VS Code. End (SIGTERM, two-click confirm) → clean SessionEnd. Folder → Explorer on `\\wsl.localhost`; VS Code → `code --remote wsl+<distro>`; both also on closed cards. All verified.
  - **No "interrupt":** SIGINT to claude ends the session too (also a clean SessionEnd). Ctrl+C/Esc in the TUI are keypresses, not signals, so stopping just the current turn would need keystroke injection, which is ruled out.
  - `AskUserQuestion` and `ExitPlanMode` (PreToolUse) now mean needs_input: no other hook says Claude is waiting on a question or plan approval.
- [x] New session from the sidebar (SPEC §3.11): "+" → recents / Other… → new WT tab via `claudes`. Recents merge sessions with the newest transcript per `~/.claude/projects` folder, resolved to project roots. Other… takes a WSL path (errors if missing) or a folder picker that opens at `\\wsl.localhost\<distro>\home`. New `terminal_window` config (wt.exe `-w`, default "0"); dev config uses a named test window so test tabs never land in the user's windows. Verified with sandbox paths only.

## M7: Install and dogfood
- [x] install.sh / uninstall.sh, Windows installer, autostart.
  - `scripts/cc_hooks.py` merges/removes our hooks (recognised by `cc-hook` in the command), atomic write + timestamped backup; `install.sh`/`uninstall.sh` wrap it and copy `cc`, `cc-hook`, `cc-recap` to `~/.local/bin`. Tested in a fake HOME with an existing Notification hook: user hook kept first, other keys untouched, re-install is a no-op, uninstall restores the original byte-for-byte JSON. The test deny rule is never installed.
  - `cc-hook` hardening for global use: with the app down a connect *times out* (Windows Firewall drops it) rather than being refused, so the shim uses a 0.3 s connect timeout and then skips sending for 15 s (0.34 s first event, 0.01 s after). Kill switch: `~/.config/cc-sidebar/off`.
  - Windows: NSIS installer (per-user), app icon, `autostart` config (release builds only; registers the installed exe).
  - At install, `config.toml` switches from the dev values (dev launcher, test hooks env, test terminal window) to `launcher = "claudes"`, `recap_command = "cc-recap"`, `terminal_window = "0"`, no `[launch_env]`.
- [x] **Get the user's approval**, then install globally. (Approved and installed 2026-10-17.)
  - Windows: NSIS per-user install to `%LOCALAPPDATA%\cc-sidebar`, Start menu shortcut, autostart (HKCU Run). No firewall prompt for the installed exe (the port rule covers it).
  - WSL: `scripts/install.sh` → `~/.local/bin/{claudes,cc-hook,cc-recap}`, hooks merged into `~/.claude/settings.json` (backup `settings.json.cc-sidebar-backup-20261017-165347`). Verified with a sandbox `claudes` session on the global hooks (card + Haiku recap); this dev session also shows up.
  - `config.toml` switched to production values (dev copy kept as `config.dev-backup.toml`; test cards in `sessions.test-backup.json`). For further sandbox testing, use `CC_SIDEBAR_SETTINGS` only for the test deny rule; hooks are global now.
- [ ] Run for a week and keep a list of issues (below).

## M8: Projects, stars, tasks (SPEC §3.12)
- [x] Hover flyout for card details (no layout shift). Opens after 300 ms, switches instantly between cards, stays while the mouse is on it; positioned with `position: fixed` below (or above) the card. Verified: cards don't move.
- [x] Starred sessions (toggle, top of project, never pruned).
- [x] Projects first class: tracked list persisted, project sections, drag to reorder, header + / ⋯ menu, close project. `projects.json`; auto-tracked when a session's project resolves; HTML5 drag-and-drop needed `dragDropEnabled: false` on the Tauri window (its native file-drop handler swallows it on Windows). Close project is refused while it has open sessions and removes its closed ones. Menu closes on outside click/Escape.
- [x] Tasks from trackers via Claude + connectors (`cc-tasks`), start a session from a task. Jira (project A) first; Azure DevOps (project B) needs a connector decision.
  - claude.ai connectors are *deferred* tools in `claude -p`: invisible in the init tool list and unreachable with `--tools ""`. `cc-tasks` therefore allows built-in `ToolSearch` only, plus the named read-only connector tools, with `--permission-mode dontAsk` (everything else refused) and `--json-schema` for the result. ~11–13 s per listing (Sonnet). No user settings, so no hooks.
  - Picking a task writes the first prompt to WSL `/tmp` and starts `claudes` with `CC_SIDEBAR_PROMPT_FILE` (read and deleted by the wrapper), so no task text crosses wt.exe's command line.
  - `projects.toml` is hot-reloaded now (names/colours/tasks). The UI caches a project's task list for 5 minutes (↻ to refresh) and filters when there are more than 6.
  - Verified with a sandbox project reading a real Jira board (8 tasks) and a harmless prompt template; the session got "received PROJ-1028".
  - `scripts/reinstall-app.ps1`: the silent installer reports success without replacing the exe if the old process hasn't fully exited; this waits and verifies.

## M9: Feedback round 2 (2026-10-17)
- [x] AskUserQuestion arrives as a PermissionRequest and showed Allow/Deny: show it as needs-input with the question, no buttons, no hold.
- [x] Project B (Azure DevOps) tasks via `az boards query` (Claude allowed only `git remote` and `az boards query`); one test against the real board approved.
  - `cc-tasks` takes permission rules (`Bash(az boards query:*)`) as well as connector tools, and runs from the project folder. In dontAsk mode Claude Code still auto-allows plain read-only commands (`ls`), but writes (`touch`) are refused.
  - Project B's folder holds several repos rather than being one, so its org/project are written into the query instead of found via `git remote`.
  - Gotcha: a WIQL with `<>` was refused despite matching the rule: Claude Code treats `<`/`>` as redirection. The query tells Claude to use `NOT IN (...)`. Result: 4 active items, 14.5 s.
- [x] Monitor selection for the sidebar (config + ⚙ menu). `monitor = 0` (primary) or n (counting from the left), each monitor's own DPI sets the width; falls back to the primary if the chosen one is gone. **Not verified on real multi-monitor hardware** (one monitor here).
- [x] Collapse to a slim docked status strip (30 DIP: expand button + one status dot per project, flashing on new alerts). Header » / strip « / tray "Collapse / expand"; remembered in `ui.json`. Verified: other windows reflow into the freed space.
- [x] Floating window mode: a normal movable, resizable window instead of the AppBar (config + ⚙ menu). Unregisters the AppBar (no reserved space), header is a drag region, frameless resize from the edges; position/size saved in `ui.json` and restored on restart (verified). `config::set_value` edits single keys in config.toml, keeping comments.

## M10: Feedback round 3 (2026-10-17)
- [x] Session flyout: no Folder/VS Code (they're on the project menu); End and Remove only.
- [x] Session names: custom title > Claude Code's `ai-title` (from the transcript) > a 3-6 word Haiku title (cc-recap's new `TITLE:` line) > first prompt. `claudes` disables Claude Code's own titling (needed for the `[cc:tag]` tab title), hence the Haiku title. Existing sessions are backfilled from their transcripts at startup and on SessionStart.
- [x] Durations show the largest unit only (5h, not 5h 36m).
- [x] A new (or /clear-ed) session that closes before any prompt is removed; resumed and starred ones stay.
- [x] ⚙ menu: one Position list (docked on monitor N, or floating).
- [x] Header + is now Add project (tracks a folder's project, no session); recents leave out tracked projects. Sessions start from a project's own +.

## M11: Desktop icons (2026-10-18)
- [x] Any change to the reserved space (collapse/expand, docked/floating, edge, width, monitor, start, quit) made Explorer re-lay out the whole desktop; with "Align to grid" the icons scattered (not just ones near the sidebar). `desktop.rs` snapshots every icon position (IShellWindows → IShellBrowser → IFolderView) right before such a change and puts them back after (two passes, 0.5 s and 1.7 s), skipping icons whose spot is now under the sidebar. Explorer re-snaps restored icons a few pixels onto its grid, so ±12 px counts as in place.
- [x] Closing the app politely (tray Quit, `taskkill` without /F, installers) releases the space and restores icons before exiting. `reinstall-app.ps1` closes it politely and force-kills only after 15 s: a force-killed AppBar can't restore, and the repeated force-kills during development are what scattered the user's icons.
- [x] Verified by the user: icons return to the right places on collapse/expand. The brief flash (Explorer moves them, then they're put back ~0.5 s later) is hidden by pausing the desktop icon view's painting (WM_SETREDRAW on its SysListView32) from just before the change until the first restore; a guard always resumes painting.

## M12: Taller cards (2026-10-18)
- [x] The card's recap shows up to 5 lines (CSS line clamp): the Haiku line followed by its detail, or, before Haiku answers, the start of the last reply (instant recap now keeps ~400 characters of plain text instead of one sentence).

## Fixes (2026-10-18)
- [x] Clicking a card left keyboard focus on the WT tab strip (UIA `Select()` focuses the tab item), so typing didn't reach the terminal. Now the tab is only selected if it isn't already active, and after switching, focus goes to the tab's visible `TermControl`. Verified: focus is the terminal and typed text reaches the right session, both when switching and when clicking the active tab.

## M13: Right-click menu, hover overlay, rename (2026-10-18)
- [x] Session right-click menu: Rename…, Copy session ID, End session (open, two-step), Remove from sidebar (closed). The webview's own context menu is suppressed (text boxes keep theirs).
- [x] Hover: the flyout is replaced by the same card drawn expanded exactly over the original (full recap, last prompt, all alerts, details). It has `pointer-events: none`, so hover follows the cards underneath: moving below the original card's bottom edge hovers the next card, and clicks on ☆ / Allow / Deny pass through to the real buttons (the top part lines up). Shifted up only if it would run off the window.
- [x] Rename: `--name` / `/rename` store a `custom-title` (+ `agent-name`) record in the transcript, and a running Claude Code re-writes its own title periodically. So: closed sessions get the records appended immediately (verified: resuming shows the new name in Claude Code's prompt box); open sessions get a sidebar name now and the records ~4 s after they close (so the exiting process can't overwrite them). Verified for open sessions too: the user renamed this session while open, closed and reopened it, and the name stuck.

## M14: Usage limits footer (2026-10-18)
- [x] Footer with the subscription limits `/usage` shows: 5-hour session and 7-day week, % used and reset time; amber ≥70 %, red ≥90 %, "Limit reached" / "Using extra usage" notes; two mini bars on the collapsed strip.
- Source: every `claude -p --output-format stream-json` call emits a `rate_limit_event` (`rate_limit_info.unifiedWindows.{five_hour,seven_day}.{utilization,resetsAt}`; documented as an Agent SDK message type, seen live with camelCase fields). `cc-recap` now runs in stream-json and adds a `USAGE:` line, so numbers refresh after every turn at no cost; `cc-usage` makes one tiny Haiku call at startup (`usage_command`, "" to skip). Considered: statusLine `rate_limits` (documented, but needs a change to the global statusLine config); the OAuth usage endpoint (undocumented, rejected).

## M15: Strip sessions, peek, closing tabs (2026-10-18)
- [x] Ending a session left its tab showing "process exited with code 143": Claude Code is the only process in tabs the sidebar opens, and WT only auto-closes on a clean exit. Launched commands end with `|| true`. Verified: End closes the tab (and the window if it was the last tab). Tabs the user opened themselves just return to their shell.
- [x] Collapsed strip lists open + starred sessions (project order), with project colour, status dot, ★ for starred and the alert flash; click = focus/resume.
- [x] Hovering a strip marker shows the session's expanded card in a separate `peek` window beside the strip (the strip is too narrow to draw it): same bundle with `?view=peek`, WS_EX_NOACTIVATE + SW_SHOWNOACTIVATE so it never takes focus, sized to the card, kept on screen. A 150 ms cursor watchdog hides it when the cursor leaves the sidebar (the webview misses quick exits). Tauri's own hide() was a no-op for a window shown through Win32, so it's hidden with ShowWindow too.

## M16: Settings window (2026-10-18)

- [x] A "settings" window (`index.html?view=settings`, created on demand), opened from ⚙ → Settings… and the tray; "Set up tasks…" opens it on the Projects tab.
- [x] Tabs:
  - **General:** docked or floating, monitor, edge, width, start with Windows, terminal window, retention.
  - **Alerts:** thresholds, toasts, a hotkey recorder, sounds with a ▶ preview and a .wav picker.
  - **Projects:** overrides plus the task source (connector or CLI).
  - **Advanced:** WSL distro, launcher, launch env, helper commands, links to open the raw files.
- [x] `settings.rs` merges the window's values into the files with toml_edit:
  - unchanged values keep their exact text, and changed ones keep their trailing comment;
  - values are compared as values, not as text;
  - projects are matched by `path`;
  - unknown top-level keys survive;
  - an unchanged file isn't rewritten.

  Validation happens on the server (enums, width 200–900, hex colours, task query, hotkey parse).

## Fixes (2026-10-19)

- [x] A session's project followed Claude's `cd`: the hook `cwd` changes with it, and a cd into a nested repo (e.g. folder-of-repos projects) re-filed the session under an auto-tracked sub-project. Its project dir is now the start dir, recovered from the transcript folder name (`session::start_dir`: the cwd or the ancestor whose `-`-encoded name matches), and saved sessions are repaired on load.
- [x] "Compacting context" stayed on the card after compaction: nothing cleared it. `SessionStart` with source `compact` now does (and restored sessions drop it).
- [x] Cards re-flashed an old alert whenever they were redrawn (hover copy, expand): each state update cancelled the timer that ends a flash, so keys stayed "fresh" for good. Flash timers now live in a ref and only stop on unmount.
- [x] Background agents and background commands are shown apart: "🤖 N agents running" (blue) and "⧗ N background tasks", hover for descriptions, listed in the expanded card. From `background_tasks` on Stop and SubagentStop (`type` "subagent" vs "shell", fixture Stop.background-shell); at Stop the agent count resyncs to the background agents.
- [x] The ⏳ waiting alert no longer adds a line or badge to the card (the status pill already says it); it still flashes the card and plays its sound.
- [x] Sounds: on Windows 11's default scheme Asterisk, Exclamation, Notification and Default all play "Windows Background.wav" (Question is silent), so most choices sounded alike. The picker now lists the Media folder's .wav files (minus alarm/ring loops), plus the scheme aliases labelled with the file each plays; new installs default to four distinct files.

## M17: Project drawer (2026-10-19)

- [x] Projects can be put away in a drawer at the bottom of the sidebar (⋯ → Put in drawer, or drag the header onto it; the drop zone shows while dragging). There they're 52 px square tiles (colour, emoji or initials, name, status dot, flash). Click a tile, or drag it onto a project, to take it out. `Tracked.shelved` is saved in projects.json. Starting a new session in a project takes it out of the drawer. In the collapsed strip, drawer projects only show sessions that need you.
- [x] Reordering shows an insertion line above or below the hovered project (by which half the cursor is in) instead of outlining it, and uses the move cursor rather than copy (`dropEffect = "move"`). `dropIndex` (unit-tested) turns before/after into the `move_project` index; the same applies to drawer tiles.
- [x] Fix: the hover overlay kept its first position when a card above it changed height. It now follows its card (placed from the card on each render, plus a ResizeObserver on the list).

## M18: Waiting on background work (2026-10-20)

- [x] Jobs Claude is waiting on (a background agent, a watcher or poll loop it will act on) get a highlighted **Waiting on** block on the card, separate from other background tasks (dev servers: "⧗ N background tasks") and from agents inside the current turn. The status pill stays as it is. (A first take, "waiting on job" as its own status, was dropped the same day: chatting with a session while its job runs made it flip to ready, which read as wrong. The old status loads as idle.)
- [x] What counts as a job: a background agent always does. A shell task is guessed from its description or command ("watch", "until", "poll", "finish", … but not "dev server", "serve", "tail -f", …), then decided by Haiku: cc-recap's `WAITING: yes|no` now asks whether background work Claude will act on is still running, even if the latest turn was about something else.
- [x] When a job from the last Stop is gone at the next Stop, and that turn wasn't started by the user, a ✅ `finished` alert goes off immediately: flash and sound (`[sounds] finished`), no card line. The trigger is the injected `<task-notification>` prompt (fixture UserPromptSubmit.task-notification).
- [x] A "ready" pill fades from green to grey between 1 and 2 hours after the session became ready (inline color-mix on the chip).
- [x] Fix: the project menu of the last project opened below the list and was clipped; it opens upwards when there's no room.
- [x] Fix: recaps for a long session were stale. Its transcript had runs of NUL bytes (a crash or sleep mid-write), plain `jq` stops at the first bad line (exit 5), so Haiku saw the first ~300 lines instead of the latest turns. That is also why it said WAITING: no for a session watching a deploy. cc-recap now reads `tail -n 800`, strips NULs and parses with `jq -R 'fromjson?'`, skipping bad lines. Tool calls show their human-written `description` before the raw command.

## M19: Ordering by drag, menus close on blur (2026-10-20)

- [x] Sessions can be dragged into order within their project, with an insertion line like projects. Saved as `Tracked.session_order` (ids). `arrange()` puts ordered ids first and the rest in the usual order (starred, then open by start), so new sessions go to the bottom. The strip follows the same order.
- [x] Drawer tiles can be reordered: drop on a tile's left or right half, shown by a vertical line. A project dragged from the list onto a tile goes into the drawer at that spot. Shared `placeProject` handles moves into, out of and within the drawer. Dropping on the drawer's empty space just puts it away.
- [x] No hover overlay during a drag (its 300 ms timer could start as the drag began and pop it up mid-drag).
- [x] Menus (session right-click, project ⋯, ⚙) close when the sidebar loses focus (window `blur`), e.g. on a click in another app.

## M20: Sessions in tmux, and the sidebar's own terminal (2026-10-20, branch `tmux-sessions`)

Built on a branch, so rolling back means installing `main` (tag `pre-tmux`) again. See SPEC §3.14.
- [x] WSL: `dev/tmux/tmux.conf` (installed by install.sh); `claudes` hosts in tmux when `CC_SIDEBAR_HOST`/`~/.config/cc-sidebar/host` says so; cc-hook sends `X-CC-Tmux`. Tested with a stub claude: arguments with spaces survive, Claude's PID is the wrapper's (liveness and End unchanged), and the title passes through.
- [x] App: `session_mode` (terminal | tmux | window); the host marker is synced on config apply; `Session.tmux`/`attached`; the attach watcher (4 s); focus falls back to a new attached tab; Hide / Open in Windows Terminal; "Running in the background" on cards.
- [x] Terminal window (`term.rs` + `TerminalWindow.tsx`): portable-pty ConPTY → `wsl.exe tmux attach`, UTF-8-safe output events, a waiter thread drops the console on exit (ConPTY only ends output then), tabs per session, and keys (copy/paste/image paste/Shift+Enter), web links, OSC 52.
- [x] Verified on screen with sandbox sessions:
  - a click opens the window attached; typing works;
  - maximising resizes the tmux client (134×44 → 301×65);
  - closing a tab detaches, the card shows "Running in the background", and a click reopens it with full history;
  - Hide closes it; Open in Windows Terminal opens an attached `[cc:<tag>]` tab, and detaching closes that tab;
  - + starts a new session detached and opens it in the window;
  - End closes the window tab, and the tmux server exits when it's the last session;
  - sessions survive the sidebar being restarted or reinstalled.
- [x] Shift+Enter: xterm.js sends a plain Enter, from the keypress too, so both are blocked and `ESC[27;2;13~` is sent (tmux parses it as S-Enter and Claude gets a newline). In Windows Terminal, Shift+Enter submits with or without tmux (checked in a plain tab), so tmux mode loses nothing there.
- [x] ConPTY asks for the cursor position (`ESC[6n`) at startup and waits for the reply; xterm.js answers it (a probe without a terminal hung until it did).
- [x] Fix (2026-10-21): the mode variable `CC_SIDEBAR_HOST=tmux` clashed with cc-hook's `CC_SIDEBAR_HOST` (the app's address override). Inside tmux sessions cc-hook posted to "tmux", failed, and kept renewing the shared app-down marker, so every session's hooks were skipped. The cards stayed "closed", and each click resumed the conversation again (three copies of one conversation ran at once). The variable is now `CC_SIDEBAR_RUN_IN`, and cc-hook ignores the stale value.
- [x] Guard: before resuming a closed card, `pgrep -f '^claude .*--resume <id>'` in WSL. If found, it doesn't resume and says it's already running.
- [x] Idle sessions with nothing showing them are ended (`Session::idle_unwatched`, checked by the 4 s attach watcher, 5 s grace for a view to attach after a click). Tested live:
  - an unwatched new session ended about 12 s after it became idle;
  - a session with `sleep 45` in the background stayed alive while it ran, then ended 6 s after reporting back, and its card shows "ready" with the ✅ alert (dormant: `closed_reason == "idle"`, treated as open by `arrange`, the strip and counts).
- [x] Resumed sessions show "ready", not "new", and the ready timer counts from the last Stop (`last_stop_at`) through an idle end and a resume, so opening or closing a session doesn't reset it.
- [x] Fix: Haiku's session-wide `WAITING: yes` marked every background command as a job, including servers (a session with a backend and a Vite dev server showed as "Waiting on 2 jobs"). `awaited()`: agents always count. Clear service words ("server", "serve", "vite", "tail -f", …) never count. Weaker ones ("backend", "frontend", "localhost") don't count unless the text speaks of waiting ("Watch the … frontend deploys" is a job). Haiku decides only the rest. Saved sessions are reclassified on load.
- [x] Removed the card's "Running in the background" line: with idle sessions ended, a session that survives without a window is working, waiting on you or has background work, all already on the card, and a click does the right thing either way.
- [x] The ✅ finished alert has no card line to dismiss, so it pulsed the project dot until the next turn. Opening the session (a click on its card or strip marker) now counts as seeing it (`Store::seen`).
- [x] Fix: Claude's status line got garbled after partial redraws in the terminal window. xterm.js's default (Unicode 6) width table counts some emoji as one column where tmux counts two. Now uses the Unicode 11 table (`@xterm/addon-unicode11`).
- [x] Fix: the expanded card's "You:" showed the `<task-notification>` that Claude Code injects when a background job finishes. Any prompt starting with a tag is now treated as injected (`injected()`), and saved cards are cleaned on load.
- [x] Fix: clicking links Claude marks as hyperlinks (OSC 8, passed through by tmux) did nothing in the terminal window: xterm.js's default handler uses confirm() and window.open. They now go through `linkHandler` to the opener, like plain URLs (web-links add-on). Both tested by clicking.
- [x] Tried on screen (2026-10-23): image paste (Ctrl+V with an image on the clipboard shows "[Image #1]"; click into the terminal first), Ctrl+B to background a running command (inside tmux Claude Code asks for it twice), copying with mouse selection (lands on the Windows clipboard), double-click on the title row to maximise.


## M21: Background task progress without asking Claude (2026-10-21)

- [x] `jobs.rs`: reads each running background task's output file (`tasks_dir` from the hooks' `scratchpad_dir`, `Background.id` from `background_tasks[].id`) every 3 s. It keeps the latest line and a URL from the output (falling back to the command and description).
- [x] Cards: the job line shows its latest output line in small monospace, with a ↗ link. Clicking a job (or the background-task count) runs `open_job_output`, which opens a `tail -f` tab in the terminal window (`term::show_command`: the window can run any registered command, not only `tmux attach`).
- [x] Backfill: items without ids are matched by description to background commands in the transcript (tool_use with `run_in_background` + its "with ID … written to …" result).
- [x] Fix: Haiku's session-wide `WAITING:` made "Start reports API …" (no service word) a job. Now:
  - cc-recap gets the running background commands as extra arguments and answers `BACKGROUND: job|service,…` for each;
  - the session keeps those labels by description (`job_labels`) while the tasks run;
  - "api" (as a whole word) joined the weaker service words.

  Checked on real transcripts: an API and a Vite server both came back as service, and a pipeline watcher as job.
- [x] Tested live on a real session's pipeline watcher (backfilled): the card showed "##### step 5, cleanup delete: run …" and the click opened its full output following live. Background servers whose output Claude redirected elsewhere have empty files, so they show nothing.

## M22: Simpler cards and session lifecycle (2026-10-21)

- [x] "Waiting on" items: no ↗ link and no click-through to the output (the hover overlay made them unreachable anyway). The latest output line stays. The command view (`term::show_command`, `open_job_output`) and URL extraction are removed.
- [x] Project headers: no status dot or flash; attention is on the sessions. Drawer tiles keep their dot.
- [x] Stars replaced by "kept until removed" (`Session.removed`, migrated once: closed and not starred → removed). Remove moves a session to the closed section and ends its process tree (`actions::end_tree`: leaves first, then `tmux kill-session`), asking first when that stops work. `/exit` removes; resuming un-removes. Pruning and Delete only apply to removed sessions. The star button is gone, which gives the title more room.
- [x] End session became **Force end** (SIGKILL tree, marked gone at once, stays in the list).

## M23: Last answer instead of Haiku summaries (2026-10-21)

- [x] The card shows Claude's last answer (`Session.last_answer` from `Stop.last_assistant_message`, plain text, no blank lines): first 5 lines on the card, 12 on hover. An ending question (`Session.question`) is pinned in bold. Old cards are backfilled from the transcript tail at startup (`last_assistant_text`).
- [x] Haiku (cc-recap) runs only for sessions with background commands to label, or no title. Other turns refresh usage via `cc-usage` when it's over 5 minutes old.

## M24: Closed sessions in a header menu (2026-10-21)

- [x] Titles stick. A session whose transcript has no stored title got a new Haiku title with every recap: the wrapper's `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` (needed to keep `[cc:<tag>]` tab titles) also stops Claude Code writing its own `ai-title`. Haiku's `TITLE:` now only names a session that has none. cc-recap reports a stored title (a /rename or Claude Code's own) as `NAME:`, which always applies.

- [x] The project header's "N open" became an "N closed" link (only when there are any), opening a menu of the project's closed sessions: name and age, a click reopens, ✕ deletes. The folded "▸ N closed" list of cards under each project is gone. The menu opens upwards for the last project, like ⋯.

## M25: Ready fade counts only time at the computer (2026-10-21)

- [x] The ready pill faded by wall-clock time, so sessions left over lunch or a night turned grey. `presence.rs` polls `GetLastInputInfo` every 10 s. No input for 5 minutes is an away spell counted from the last input, which also covers a locked screen and sleep. Spells are saved to `presence.json` (with `last_input`, so time the app or computer was off counts as away) and sent in the UI state. The fade (1–2 h) uses `presentTime()`, time since ready minus away spells. The pill's timer still shows real time.

## M26: Minimised session cards (2026-10-21)

- [x] Follow-up: hover and minimise clashed (the preview, drawn expanded over a minimised card, hid its ▸). Now the title bar is for controls and the body for the preview: hovering the title bar never opens the preview, and a fold toggle (▾/▸) sits in the header's left padding, always shown on minimised cards and on hover otherwise, so one click folds or unfolds. Minimised cards, all title bar, have no preview. Tested on screen.
- [x] `Session.minimized` (saved). A minimised card shows only its title bar, with a ▸ before the name that expands it. Right-click → Minimise/Expand; the project ⋯ → Minimise all/Expand all. It opens by itself while the session needs the user (permission, question, error), and the hover copy shows everything. Not double-click: a click already opens the session. Tested on screen with the dev session's own card.

## M27: No header; controls in the footer (2026-10-22)

- [x] Fix (2026-10-26): finished turns turned to "your turn" a minute later. Claude Code now sends the `idle_prompt` notification after a minute at the prompt, and it counted as needs-input. It's ignored now.
- [x] The terminal window looks like Windows Terminal (2026-10-23): no title bar; the tabs, drag space and caption buttons (Segoe Fluent Icons, red close on hover) share one 40 px row. Tested on screen: maximise fits beside the sidebar, restore, drag by the tab row, close.
- [x] Fix (2026-10-23): closing a new session before any prompt left its card up ~10 s (the 4 s attach poll plus the 5 s grace). Closing a view now marks it closed and wakes the attach check; a never-prompted session with a closed view ends at once. Tested on screen: gone 1 s after the tab closed.
- [x] Fix: a pipeline watcher ("Follow the … optimisation through dispatch, ML job and ingestion") showed as a background task although Haiku labelled it a job. Its script's `api-version=` matched the weak service word "api", and the keyword guard overrode the label. Haiku's per-task label now wins outright. Keyword guesses read the description, plus only unmistakable server commands from the command. "follow" and "track" joined the waiting words.

- [x] The "cc-sidebar" title and "N open" count are gone. The +, ⚙ and collapse buttons sit right of the usage bars in one footer, which is the floating window's drag handle. The ⚙ menu opens upwards from it, and the add-project panel appears just above it.

## M28: Prompt cache and idle compaction (2026-10-26)

- [x] SPEC §3.15. Cards show the prompt cache's countdown, "expired · next prompt re-sends N", and "Compacted while idle · pre → post", read from transcripts every 30 s; details on hover. Seen on screen with the live sessions (an expired 538k cache; three idle compactions).
- [x] The ready pill follows the cache: green while warm, grey once expired or compacted; with a 5-minute cache or none, grey after an hour. No fade; presence tracking (`presence.rs`, GetLastInputInfo, `presence.json`) removed.
- [ ] Unverified: whether the PreCompact hook fires for idle compaction (the code path suggests it does). Testing needs a 200k+ context left idle for 54 min.

## M29: PRs on cards (2026-10-26)

- [x] SPEC §3.16: PRs a session opened (GitHub and Azure DevOps), with state, checks and review on the card, and alerts for failing checks, changes requested and merges. Tested against live data: 30 PRs found in three sessions, all looked up; the open one shows on its card.
- [x] PRs moved to a footer band, one line each, kept on minimised cards, labelled with the project folder holding the repo (from `.git/config` remotes). Seen on screen.
- [x] The "Waiting on" box became a blue footer band above the PRs (name + output; name only when minimised); the background-task count stays in the body. Seen on screen.
- [x] Fixes from that test: Azure DevOps commands without `--org`, or after a heredoc with an apostrophe, lost the organisation; a PR found twice stayed unchecked; a PR first looked up after a failed attempt raised a false "merged" alert.

## M30: Card structure (2026-10-26)

- [x] Alerts moved from the body to an alert band above the status band (option 1 of three mock-ups), "+N" expands to a list with ✕ each and Dismiss all; kept on minimised cards. Tested on screen with a sandbox card given two alerts by posted hook events: "+1", the expanded list, ✕ on one alert, back to a single line.
- [x] Fix: a session that ended while ready (its tab closed) reset its ready timer; only the sidebar's idle end kept it. Now any end while ready keeps it.
- [x] Turn view while working (concept 2 of 3): prompt, task-list progress, recent steps, current action; no old answer or question. Seen on screen with the dev session's own card. No session used a task-list tool in the last 10 days, so the progress bar is untested live.
- [x] SPEC §3.17: status as a flush block in the title bar; counts, cache, compaction and alert badges in a status band at the bottom (concept B of two rendered mock-ups). Seen on screen.

## M31: Floating as a normal window (2026-10-26)

- [x] Floating mode is a normal window: title bar, taskbar button, minimise/maximise, not on top; maximised state remembered. No collapse while floating. The strip's expand button moved to the bottom. Usage bars capped at 360 px. Maximise and restore tested on screen.
- [x] Wide layout (concept 2 of 3 mock-ups): projects in masonry columns from 760 px, drawer as a side column. Keyboard navigation (arrows, Enter, A/D, Esc). Seen on screen in the user's floating window: two columns, side drawer; arrows moved down a column and across. Rejected for now: a "needs you" lane, bigger cards in wide mode, a docked terminal.

## M32: Job progress (2026-10-30)

Generic progress for background jobs (SPEC §3.18): the job reports it, the sidebar renders it. Option chosen from four (sidebar polls the provider itself; Claude reports via a helper; Haiku summarises the output; elapsed time only): the helper, plus elapsed time as the fallback. Haiku summaries not needed for now.

- [x] Elapsed time and "quiet N" on job lines (first-seen time; output file mtime).
- [x] `dev/bin/cc-progress`: prints `##cc-progress {json}`, always exits 0, skips unchanged lines per item and parent PID. Installed by `install.sh`.
- [x] `jobs.rs` parses progress lines (merged per item); `last_line` skips them; a task with progress is a job.
- [x] Card: steps, bar or state per item, ↗ link, colours; blocked/failed alerts.
- [x] Background agents: their TodoWrite becomes their job line's progress.
- [x] SessionStart answered with `additionalContext` describing `cc-progress`.
- [x] Sandbox test (headless sessions given the same SessionStart context through a project-local hook): unprompted, a deploy watcher reported its stages (`--steps … --at … --state`, done at the end, with the run URL), a 25-file batch reported `--done n --total 25`, and a static file server didn't use it. Repeats were skipped (4 progress lines in 14 polls).
- [x] Seen on screen (installed app, sandbox `claude -p` session given the context by the app itself): the job line read `Build ✓ ─ Dev ✓ ─ TST ⏸ · awaiting approval ↗` in amber with the timer and output line, and the 🚦 alert raised.
- [x] Fix from that test: the 🚦 alert stayed after the deploy finished. Only blocked raises it now, and it goes once nothing running is blocked (also on load). Failed only turns the line red.
- [x] Fix: headless sessions (`claude -p`) send no `scratchpad_dir`, so their jobs' output was never read. The output folder now comes from the transcript when missing.

## Fixes (2026-10-27)

- [x] An artifact's "live updates" subscription showed as a job (Haiku's label). Commandless subscriptions ("live updates for …", "re-armed on session resume") are always background tasks.
- [x] A session waiting on background agents showed "working" with the prompt free: each agent's tool call fired PreToolUse/PostToolUse for the session. Hooks carry `agent_id` inside a subagent; a background agent's tool calls no longer move the status (its permission requests still do).

## Fixes (2026-10-28)

- [x] After a machine restart, sessions found gone timed from the boot. A session found gone keeps its ready timer, or else counts from its last event; saved ones are repaired on load.
- [x] Switching a maximised floating window to docked left it full-screen and black. It is restored before docking (floating keeps its maximised state).
- [x] Claude Code's idle compaction (shortly before a large context's cache expires) read "Context full: compacting". An automatic compaction while the session is ready now reads "Compacting while idle, before the cache expires".

## Housekeeping (2026-10-17, v0.2.0)
- [x] Test data cleared from the sidebar (sandbox projects/sessions) and `%APPDATA%\cc-sidebar` (dev backups).
- [x] Saved projects re-read `projects.toml` at startup (a project renamed in projects.toml kept showing its folder name).
- [x] Frontend logic in `src/lib/view.ts` with Vitest tests (`npm test`); sections and strip share `projectStatus`.
- [x] `scripts/release.sh [version]`: all checks → installer → upgrade the installed app → update WSL scripts. v0.2.0 released with it.
- [ ] Broad firewall Allow rules for `hook-listener.exe` and the debug `cc-sidebar.exe` (from the M1/M2 prompts) need removing from an admin shell; the port rule covers everything.

## Dogfood issues
- 2026-10-17: two sessions open since before the install didn't appear. They were simply idle (no events until used); after a restart they showed. (My first guess, that older builds only read hooks at startup, was wrong.) Idea: show running-but-silent `claude` processes as greyed cards.
- 2026-10-17: click-to-focus fails for sessions started with plain `claude`: their tab titles are Claude Code's task summaries, never the project name, so the §3.8 fallback almost never matches. Now says so explicitly ("restart with `claudes --continue`") and the card notes it. Suggested `alias claude=claudes` in ~/.bashrc so new sessions are always tagged (the user to decide). Tagged sessions focus fine in the installed build (verified with a sandbox tab, including restore from minimised).
- 2026-10-17: a session last run via `claudes`, then resumed with plain `claude`, showed up and then went to Closed ("process gone") after ~30 s. Resume keeps the session_id, plain `claude` sends no PID, and the card kept the old run's tag/PID, so liveness saw the dead PID. Fixed: SessionStart (a new process) now replaces tag, PID and WT session with whatever it reports, including none. Verified with a sandbox session past a liveness check.

- 2026-10-23: once, right after Ctrl+B backgrounded a command and the turn ended, a sandbox card showed "working" with the activity "Placeholder" (a PreToolUse for a tool of that name, not in the transcript) until the job finished. Not reproduced with a payload log on the next try; maybe Claude Code's speculative run of a suggested prompt. If it recurs, ignore PreToolUse for `Placeholder`.

## Backlog (not started; see the next-steps list in chat 2026-10-17)
- Desktop icon flash on collapse/expand (2026-10-18): shorter since painting is paused, but still visible. Likely causes: Explorer repaints the desktop (wallpaper window / list view) on the work-area change before or around the WM_SETREDRAW pause, or the first restore pass (0.5 s) comes after Explorer's own repaint. Ideas: restore as soon as a position change is detected (poll every ~50 ms) instead of after a fixed delay; also pause the parent SHELLDLL_DefView / Progman; check whether the icons move before the AppBar change takes effect.
- Silent sessions: show running `claude` processes that send no hooks as greyed cards.
- Recap cost controls (only after longer turns, or on demand).
- Send-prompt if the org enables Channels.
