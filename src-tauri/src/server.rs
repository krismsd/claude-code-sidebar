//! Hook server (SPEC §3.2): receives events from the `cc-hook` shim on the WSL adapter address
//! (WSL runs in NAT mode) and on loopback, feeds the store, and pushes snapshots to the UI.
use crate::alerts::{self, Notifier};
use crate::config::{self, Config};
use crate::events::{Event, Hook, HookMeta};
use crate::project::{self, Overrides};
use crate::session::Ms;
use crate::store::Store;
use axum::{Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

pub const PORT: u16 = 47821;
/// How long a PermissionRequest is held. Just under the shim's curl limit (cc-hook 580).
const PERMISSION_HOLD: Duration = Duration::from_secs(575);

pub type Shared = Arc<Mutex<Store>>;
/// Held PermissionRequests by session: (request number, channel for the card's allow/deny).
pub type Holds = Arc<Mutex<HashMap<String, (u64, tokio::sync::oneshot::Sender<bool>)>>>;

#[derive(Clone)]
pub struct Ctx {
    pub store: Shared,
    pub app: AppHandle,
    /// projects.toml, hot-reloaded.
    pub overrides: Arc<RwLock<Overrides>>,
    pub config: Arc<RwLock<Config>>,
    /// Sidebar window handle, for re-docking on config changes.
    pub hwnd: isize,
    pub notifier: Arc<Mutex<Notifier>>,
    pub recapper: crate::recap::Recapper,
    pub holds: Holds,
    /// Collapsed state and floating window position (ui.json).
    pub prefs: Arc<Mutex<crate::ui::UiPrefs>>,
    /// Latest subscription usage limits, if known.
    pub usage: Arc<Mutex<Option<crate::usage::Usage>>>,
    /// Check tmux attachments now rather than at the next tick (a view was closed).
    pub recheck: Arc<tokio::sync::Notify>,
}

static HOLD_SEQ: AtomicU64 = AtomicU64::new(1);

/// The card's answer to a held PermissionRequest. Err if nothing is held for the session.
pub fn answer_permission(ctx: &Ctx, session_id: &str, allow: bool) -> Result<(), String> {
    let (_, tx) = ctx
        .holds
        .lock()
        .unwrap()
        .remove(session_id)
        .ok_or("That permission request is no longer waiting.")?;
    tx.send(allow)
        .map_err(|_| "That permission request is no longer waiting.".to_string())
}

fn decision(allow: bool) -> String {
    let d = if allow {
        serde_json::json!({ "behavior": "allow" })
    } else {
        serde_json::json!({ "behavior": "deny", "message": "Denied by the user from cc-sidebar." })
    };
    serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": d } })
        .to_string()
}

pub fn now_ms() -> Ms {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as Ms)
        .unwrap_or(0)
}

/// Push the current ordered session list to the UI.
/// Everything the UI shows: tracked projects (in order) and all sessions.
#[derive(Clone, serde::Serialize)]
pub struct UiState {
    pub projects: Vec<crate::store::Tracked>,
    pub sessions: Vec<crate::session::Session>,
    /// Roots of projects with `[project.tasks]` set up (show "Tasks…").
    pub task_projects: Vec<String>,
    pub collapsed: bool,
    pub floating: bool,
    pub left_edge: bool,
    /// Configured monitor: 0 = primary, n = nth from the left.
    pub monitor: usize,
    pub usage: Option<crate::usage::Usage>,
}

pub fn ui_state(ctx: &Ctx) -> UiState {
    let store = ctx.store.lock().unwrap();
    let overrides = ctx.overrides.read().unwrap();
    let projects = store.projects().to_vec();
    let task_projects = projects
        .iter()
        .map(|t| t.project.root.clone())
        .filter(|r| overrides.tasks_for(r).is_some())
        .collect();
    let cfg = ctx.config.read().unwrap();
    UiState {
        projects,
        sessions: store.snapshot(),
        task_projects,
        collapsed: ctx.prefs.lock().unwrap().collapsed,
        floating: cfg.floating(),
        left_edge: cfg.edge.eq_ignore_ascii_case("left"),
        monitor: cfg.monitor,
        usage: ctx.usage.lock().unwrap().clone(),
    }
}

/// Push the current state to the UI.
pub fn publish(ctx: &Ctx) {
    let state = ui_state(ctx);
    let _ = ctx.app.emit("state", state);
}

/// Sound and toast for anything newly alerting (SPEC §3.3).
pub fn announce(ctx: &Ctx) {
    let news = {
        let store = ctx.store.lock().unwrap();
        ctx.notifier.lock().unwrap().scan(&store, false)
    };
    if news.is_empty() {
        return;
    }
    let cfg = ctx.config.read().unwrap().clone();
    #[cfg(windows)]
    let hidden = crate::appbar::covered();
    #[cfg(not(windows))]
    let hidden = false;
    // One sound per batch (the first), so a burst doesn't stack up.
    #[cfg(windows)]
    if let Some(sound) = news
        .iter()
        .find_map(|n| cfg.sounds.get(n.sound_key).filter(|s| !s.is_empty()))
    {
        alerts::play(sound);
    }
    if alerts::wants_toast(&cfg, hidden) {
        use tauri_plugin_notification::NotificationExt;
        for n in &news {
            let _ = ctx
                .app
                .notification()
                .builder()
                .title(&n.title)
                .body(&n.body)
                .show();
        }
    }
}

/// Cleans up a held PermissionRequest. If it's dropped before we answer, that's Claude Code
/// killing the hook because the user answered in the terminal (SPEC §3.6).
struct HoldGuard {
    ctx: Ctx,
    session_id: String,
    seq: u64,
    answered: bool,
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        let mut holds = self.ctx.holds.lock().unwrap();
        if holds
            .get(&self.session_id)
            .is_some_and(|(n, _)| *n == self.seq)
        {
            holds.remove(&self.session_id);
        }
        drop(holds);
        let mut store = self.ctx.store.lock().unwrap();
        store.permission_holdable(&self.session_id, false);
        if !self.answered {
            store.permission_released(&self.session_id, now_ms());
        }
        drop(store);
        publish(&self.ctx);
    }
}

async fn hook(State(ctx): State<Ctx>, headers: HeaderMap, body: Bytes) -> String {
    let meta = HookMeta::from_headers(|k| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    });
    let hook = match Hook::parse(&body, meta) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("bad hook payload: {e}");
            return String::new();
        }
    };
    let applied = ctx.store.lock().unwrap().apply(&hook, now_ms());
    if !applied {
        return String::new();
    }
    publish(&ctx);
    announce(&ctx);
    refresh_project(&ctx, &hook);
    if matches!(hook.event, Event::Stop { .. }) {
        // The card shows Claude's own last answer. Haiku is only needed for what it alone can
        // tell: whether background commands are jobs or services, and a title when Claude Code
        // hasn't set one. Otherwise just keep the usage numbers fresh (a tinier call).
        let needs_haiku = ctx
            .store
            .lock()
            .unwrap()
            .get(&hook.common.session_id)
            .is_some_and(|s| {
                (s.title.is_none() && s.user_title.is_none())
                    || s.background.iter().any(|b| !b.agent)
            });
        if needs_haiku {
            ctx.recapper.on_stop(&ctx, &hook.common.session_id);
        } else {
            refresh_usage_if_stale(&ctx);
        }
    }
    if matches!(hook.event, Event::SessionStart { .. }) {
        backfill_names(&ctx);
    }
    if matches!(hook.event, Event::SessionEnd { .. }) {
        sync_titles(&ctx, Duration::from_secs(4));
    }

    // Questions for the user aren't allow/deny decisions: answer only in the terminal.
    if let Event::PermissionRequest { tool_name, .. } = &hook.event
        && tool_name != "AskUserQuestion"
    {
        // Hold the request: the card can answer it, a terminal answer shows up as a dropped
        // connection (guard), and on timeout we release it with no decision.
        let id = hook.common.session_id.clone();
        let seq = HOLD_SEQ.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        ctx.holds.lock().unwrap().insert(id.clone(), (seq, tx));
        ctx.store.lock().unwrap().permission_holdable(&id, true);
        publish(&ctx);
        let mut guard = HoldGuard {
            ctx: ctx.clone(),
            session_id: id.clone(),
            seq,
            answered: false,
        };
        let answer = tokio::time::timeout(PERMISSION_HOLD, rx).await;
        guard.answered = true;
        return match answer {
            Ok(Ok(allow)) => {
                ctx.store.lock().unwrap().permission_answered(&id, now_ms());
                log::info!(
                    "permission {} from the card",
                    if allow { "allowed" } else { "denied" }
                );
                decision(allow)
            }
            _ => "{}".into(),
        };
    }
    String::new()
}

/// Resolve the session's project when first seen, and re-read the branch on Stop (it may
/// have switched branches). Runs off the request path; files are read over \\wsl.localhost.
fn refresh_project(ctx: &Ctx, hook: &Hook) {
    let (cwd, distro, known) = {
        let store = ctx.store.lock().unwrap();
        let Some(s) = store.get(&hook.common.session_id) else {
            return;
        };
        (
            s.cwd.clone(),
            s.distro.clone(),
            s.project.as_ref().map(|p| p.root.clone()),
        )
    };
    let Some(cwd) = cwd else { return };
    let needed =
        known.is_none() || matches!(hook.event, Event::Stop { .. } | Event::SessionStart { .. });
    if !needed {
        return;
    }
    let ctx = ctx.clone();
    let id = hook.common.session_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let distro = distro.unwrap_or_else(|| "Ubuntu".into());
        let overrides = ctx.overrides.read().unwrap().clone();
        let p = project::resolve(&cwd, |l| project::windows_path(&distro, l), &overrides);
        ctx.store.lock().unwrap().set_project(&id, p);
        publish(&ctx);
    });
}

/// Re-resolve every session's project (names, colours, emoji from projects.toml). Also run at
/// startup, since projects.toml may have changed while the app was closed.
pub fn refresh_all_projects(ctx: &Ctx) {
    let todo: Vec<(String, String, String)> = ctx
        .store
        .lock()
        .unwrap()
        .snapshot()
        .into_iter()
        .filter_map(|s| {
            Some((
                s.session_id,
                s.cwd?,
                s.distro.unwrap_or_else(|| "Ubuntu".into()),
            ))
        })
        .collect();
    let ctx = ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let overrides = ctx.overrides.read().unwrap().clone();
        for (id, cwd, distro) in todo {
            let p = project::resolve(&cwd, |l| project::windows_path(&distro, l), &overrides);
            ctx.store.lock().unwrap().set_project(&id, p);
        }
        publish(&ctx);
    });
}

/// Name sessions that have none yet from their transcripts (read over \\\\wsl.localhost): sessions
/// from before names existed, or ones the app first sees mid-way.
pub fn backfill_names(ctx: &Ctx) {
    let todo = ctx.store.lock().unwrap().unnamed();
    if todo.is_empty() {
        return;
    }
    let ctx = ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let default_distro = ctx.config.read().unwrap().distro.clone();
        for (id, path, distro) in todo {
            let file = project::windows_path(distro.as_deref().unwrap_or(&default_distro), &path);
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let (title, first) = crate::session::transcript_names(&text);
            ctx.store.lock().unwrap().set_names(&id, title, first);
        }
        publish(&ctx);
    });
}

/// Cards with no last answer yet (data from before it was kept): read it from the transcript,
/// the last assistant text there. Only the end of the file is read.
pub fn backfill_answers(ctx: &Ctx) {
    let todo = ctx.store.lock().unwrap().unanswered();
    if todo.is_empty() {
        return;
    }
    let ctx = ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::{Read, Seek, SeekFrom};
        let default_distro = ctx.config.read().unwrap().distro.clone();
        for (id, path, distro) in todo {
            let file = project::windows_path(distro.as_deref().unwrap_or(&default_distro), &path);
            let Ok(mut f) = std::fs::File::open(&file) else {
                continue;
            };
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = f.seek(SeekFrom::Start(len.saturating_sub(1 << 20)));
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_err() {
                continue;
            }
            if let Some(msg) = crate::session::last_assistant_text(&String::from_utf8_lossy(&buf)) {
                ctx.store.lock().unwrap().set_answer(&id, &msg);
            }
        }
        publish(&ctx);
    });
}

/// Write sidebar names of closed sessions into their transcripts as `custom-title` records
/// (what `claude --name` / `/rename` write), so Claude Code's /resume picker shows them. Waits
/// a few seconds first: an exiting Claude Code re-writes its own title on the way out.
pub fn sync_titles(ctx: &Ctx, delay: Duration) {
    let ctx = ctx.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let todo = ctx.store.lock().unwrap().unsynced_titles();
        let distro = ctx.config.read().unwrap().distro.clone();
        for s in todo {
            let (Some(path), Some(name)) = (&s.transcript_path, &s.user_title) else {
                continue;
            };
            let file = project::windows_path(s.distro.as_deref().unwrap_or(&distro), path);
            match append_title(&file, &s.session_id, name) {
                Ok(()) => ctx.store.lock().unwrap().mark_title_synced(&s.session_id),
                Err(e) => log::warn!("rename {}: {e}", s.session_id),
            }
        }
    });
}

/// Append the two records `claude --name` writes (Claude Code reads the latest).
pub fn append_title(file: &std::path::Path, session_id: &str, name: &str) -> std::io::Result<()> {
    use std::io::Write;
    let records = [
        serde_json::json!({ "type": "custom-title", "customTitle": name, "sessionId": session_id }),
        serde_json::json!({ "type": "agent-name", "agentName": name, "sessionId": session_id }),
    ];
    let mut f = std::fs::OpenOptions::new().append(true).open(file)?;
    // Start on a fresh line even if the file doesn't end with one.
    let text: String = records.iter().map(|r| format!("{r}\n")).collect();
    let needs_newline = std::fs::read(file)
        .map(|b| b.last().is_some_and(|c| *c != b'\n'))
        .unwrap_or(false);
    f.write_all(format!("{}{text}", if needs_newline { "\n" } else { "" }).as_bytes())
}

/// New usage numbers (a `rate_limit_info` JSON object): keep them and update the UI.
pub fn set_usage(ctx: &Ctx, json: &str) {
    match crate::usage::parse(json, now_ms()) {
        Some(u) => {
            *ctx.usage.lock().unwrap() = Some(u);
            publish(ctx);
        }
        None => log::warn!("usage: couldn't read {json}"),
    }
}

/// Usage numbers older than this are refreshed after a turn that didn't need Haiku.
const USAGE_STALE_MS: Ms = 5 * 60_000;

fn refresh_usage_if_stale(ctx: &Ctx) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let fresh = ctx
        .usage
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|u| now_ms().saturating_sub(u.updated_at) < USAGE_STALE_MS);
    if fresh || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let ctx = ctx.clone();
    tauri::async_runtime::spawn(async move {
        fetch_usage(&ctx).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// At startup: one tiny `claude -p` call (`cc-usage`) so the usage footer has numbers before
/// the first recap. Later they come with each recap, or from another such call when stale.
pub async fn probe_usage(ctx: Ctx) {
    tokio::time::sleep(Duration::from_secs(3)).await;
    fetch_usage(&ctx).await;
}

async fn fetch_usage(ctx: &Ctx) {
    let (cmd_name, distro) = {
        let cfg = ctx.config.read().unwrap();
        (cfg.usage_command.clone(), cfg.distro.clone())
    };
    if cmd_name.is_empty() {
        return;
    }
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(["-d", &distro, "-e", "bash", "-lc", &cmd_name]);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = text.lines().rev().find(|l| l.trim_start().starts_with('{')) {
                set_usage(ctx, line);
            }
        }
        _ => log::warn!("usage: {cmd_name} failed"),
    }
}

/// IPv4 address of the Windows side of the WSL NAT network ("vEthernet (WSL …)").
pub fn wsl_host_ip() -> Option<IpAddr> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find(|i| i.name.contains("WSL") && i.ip().is_ipv4())
        .map(|i| i.ip())
}

/// Serve on loopback plus the WSL adapter, re-checking the adapter address every 30 s (it can
/// change across reboots or WSL restarts).
pub async fn run(ctx: Ctx) {
    let router = Router::new()
        .route("/hook", post(hook))
        .with_state(ctx.clone());
    let mut bound: HashSet<IpAddr> = HashSet::new();
    loop {
        let mut want = vec![IpAddr::from([127, 0, 0, 1])];
        want.extend(wsl_host_ip());
        for ip in want {
            if bound.contains(&ip) {
                continue;
            }
            match tokio::net::TcpListener::bind((ip, PORT)).await {
                Ok(listener) => {
                    log::info!("hook server listening on {ip}:{PORT}");
                    bound.insert(ip);
                    let r = router.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = axum::serve(listener, r).await {
                            log::error!("hook server on {ip} stopped: {e}");
                        }
                    });
                }
                Err(e) => log::warn!("can't bind {ip}:{PORT}: {e}"),
            }
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Once a second: time-based transitions (alerts, pruning). When anything changed, publish it
/// and save it to `sessions.json` so closed sessions survive restarts.
pub async fn ticker(ctx: Ctx) {
    let mut last = String::new();
    let mut last_projects = String::new();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (json, projects) = {
            let mut store = ctx.store.lock().unwrap();
            store.tick(now_ms());
            (
                serde_json::to_string(&store.snapshot()).unwrap_or_default(),
                serde_json::to_string(store.projects()).unwrap_or_default(),
            )
        };
        // Floating window moved/resized: remember it (only while expanded, so the strip
        // doesn't overwrite the expanded size).
        #[cfg(windows)]
        if let Some(rect) = crate::appbar::take_moved() {
            let mut prefs = ctx.prefs.lock().unwrap();
            if !prefs.collapsed {
                prefs.float_rect = Some(rect);
                prefs.save();
            }
        }
        if json != last || projects != last_projects {
            publish(&ctx);
            announce(&ctx);
            save("sessions.json", &json);
            save("projects.json", &projects);
            last = json;
            last_projects = projects;
        }
    }
}

fn data_path(name: &str) -> Option<std::path::PathBuf> {
    config::dir().map(|d| d.join(name))
}

/// Write atomically (temp file + rename) so a crash never leaves a torn file.
fn save(name: &str, json: &str) {
    let Some(path) = data_path(name) else { return };
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_err()
    {
        log::warn!("couldn't save {}", path.display());
    }
}

/// Load saved sessions and tracked projects (app start).
pub fn load_saved(store: &mut Store) {
    let read = |name| data_path(name).and_then(|p| std::fs::read_to_string(p).ok());
    if let Some(text) = read("sessions.json") {
        match serde_json::from_str(&text) {
            Ok(saved) => store.restore(saved, now_ms()),
            Err(e) => log::warn!("sessions.json: {e}"),
        }
    }
    match read("projects.json").map(|t| serde_json::from_str(&t)) {
        Some(Ok(saved)) => store.restore_projects(saved),
        Some(Err(e)) => log::warn!("projects.json: {e}"),
        // First run with projects: track whatever the saved sessions belong to.
        None => {
            for s in store.snapshot() {
                if let Some(p) = s.project {
                    store.set_project(&s.session_id, p);
                }
            }
        }
    }
}

/// Push config values into the store and the AppBar.
pub fn apply_config(ctx: &Ctx) {
    let cfg = ctx.config.read().unwrap().clone();
    {
        let mut store = ctx.store.lock().unwrap();
        store.thresholds.waiting_ms = cfg.waiting_minutes * 60_000;
        store.thresholds.slow_ms = cfg.slow_minutes * 60_000;
        store.closed_retention_ms = cfg.closed_retention_days * 24 * 3_600_000;
    }
    // Autostart registers this exe's path, so only installed (release) builds do it.
    if !cfg!(debug_assertions) {
        use tauri_plugin_autostart::ManagerExt;
        let al = ctx.app.autolaunch();
        let on = al.is_enabled().unwrap_or(false);
        if cfg.autostart != on {
            let r = if cfg.autostart {
                al.enable()
            } else {
                al.disable()
            };
            if let Err(e) = r {
                log::warn!("autostart: {e}");
            }
        }
    }
    // Tell `claudes` in WSL whether to host sessions in tmux.
    #[cfg(windows)]
    {
        let cfg = cfg.clone();
        tauri::async_runtime::spawn_blocking(move || crate::actions::sync_host(&cfg));
    }
    #[cfg(windows)]
    {
        crate::appbar::configure(
            windows::Win32::Foundation::HWND(ctx.hwnd as *mut _),
            placement(&cfg, &ctx.prefs.lock().unwrap()),
        );
        // Floating windows can be resized (the frameless window gets resize borders).
        if let Some(w) = tauri::Manager::get_webview_window(&ctx.app, "main") {
            let _ = w.set_resizable(cfg.floating() && !ctx.prefs.lock().unwrap().collapsed);
        }
    }
    publish(ctx);
}

/// Width of the collapsed status strip (DIP).
pub const STRIP_WIDTH: i32 = 30;

#[cfg(windows)]
pub fn placement(cfg: &Config, prefs: &crate::ui::UiPrefs) -> crate::appbar::Placement {
    crate::appbar::Placement {
        left_edge: cfg.edge.eq_ignore_ascii_case("left"),
        width: if prefs.collapsed {
            STRIP_WIDTH
        } else {
            cfg.width.clamp(200, 1200)
        },
        monitor: cfg.monitor,
        floating: cfg.floating(),
        collapsed: prefs.collapsed,
        float_rect: prefs.float_rect,
    }
}

/// Collapse to the status strip or expand back.
pub fn set_collapsed(ctx: &Ctx, collapsed: bool) {
    {
        let mut prefs = ctx.prefs.lock().unwrap();
        prefs.collapsed = collapsed;
        prefs.save();
    }
    apply_config(ctx);
}

/// Hot reload: poll config.toml's modified time every 2 s.
pub async fn watch_config(ctx: Ctx) {
    let mut seen = Config::modified();
    let mut seen_projects = Overrides::modified();
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let projects_now = Overrides::modified();
        if projects_now != seen_projects {
            seen_projects = projects_now;
            *ctx.overrides.write().unwrap() = Overrides::load();
            log::info!("projects.toml reloaded");
            refresh_all_projects(&ctx);
        }
        let now = Config::modified();
        if now != seen {
            seen = now;
            *ctx.config.write().unwrap() = Config::load();
            log::info!("config reloaded");
            apply_config(&ctx);
            let _ = ctx.app.emit("config-reloaded", ());
        }
    }
}
