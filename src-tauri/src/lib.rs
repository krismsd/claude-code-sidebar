use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, State};

#[cfg(windows)]
mod actions;
mod alerts;
#[cfg(windows)]
mod appbar;
mod config;
#[cfg(windows)]
mod desktop;
pub mod events;
#[cfg(windows)]
mod focus;
mod liveness;
pub mod peek;
pub mod project;
mod recap;
mod server;
pub mod session;
mod settings;
pub mod store;
mod tasks;
#[cfg(test)]
mod tests;
mod ui;
mod usage;

use server::{Ctx, Shared};

#[tauri::command]
fn get_state(ctx: State<'_, Ctx>) -> server::UiState {
    server::ui_state(&ctx)
}

/// List a project's tasks (SPEC §3.12). Slow: it's a Claude call (~10 s).
#[tauri::command]
async fn list_tasks(ctx: State<'_, Ctx>, root: String) -> Result<Vec<tasks::Task>, String> {
    let tc = ctx
        .overrides
        .read()
        .unwrap()
        .tasks_for(&root)
        .cloned()
        .ok_or("No tasks set up for this project (projects.toml).")?;
    let cfg = ctx.config.read().unwrap().clone();
    tasks::list(&cfg, &root, &tc).await
}

/// Start a session in the project with the task as its first prompt.
#[cfg(windows)]
#[tauri::command]
fn start_task(ctx: State<'_, Ctx>, root: String, task: tasks::Task) -> Result<(), String> {
    let cfg = ctx.config.read().unwrap().clone();
    let overrides = ctx.overrides.read().unwrap();
    let name = ctx
        .store
        .lock()
        .unwrap()
        .projects()
        .iter()
        .find(|t| t.project.root == root)
        .map(|t| t.project.name.clone())
        .unwrap_or_else(|| root.rsplit('/').next().unwrap_or("claude").to_string());
    tasks::start(&cfg, &root, &name, &task, overrides.tasks_for(&root))
}

/// Open projects.toml (created from a commented template the first time).
#[tauri::command]
fn open_projects_file(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = project::Overrides::path().ok_or("no APPDATA")?;
    if !path.exists() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        std::fs::write(&path, project::PROJECTS_TEMPLATE).map_err(|e| e.to_string())?;
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Name a session in the sidebar ("" clears it). Closed sessions get it written to their
/// transcript right away; open ones when they close.
#[tauri::command]
fn rename_session(ctx: State<'_, Ctx>, session_id: String, name: String) {
    let closed = ctx.store.lock().unwrap().rename(&session_id, &name);
    server::publish(&ctx);
    if closed.is_some() {
        server::sync_titles(&ctx, std::time::Duration::ZERO);
    }
}

/// Collapsed strip: show / size / hide the peek card for a session (see `peek`).
#[tauri::command]
fn peek_show(
    app: tauri::AppHandle,
    ctx: State<'_, Ctx>,
    session_id: String,
    top: f64,
) -> Result<(), String> {
    let left = ctx.config.read().unwrap().edge.eq_ignore_ascii_case("left");
    peek::show(&app, &session_id, top, left)
}

#[tauri::command]
fn peek_resize(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    peek::resize(&app, height)
}

#[tauri::command]
fn peek_hide(app: tauri::AppHandle) {
    peek::hide(&app);
}

#[tauri::command]
fn toggle_star(ctx: State<'_, Ctx>, session_id: String) {
    if ctx.store.lock().unwrap().toggle_star(&session_id) {
        server::publish(&ctx);
    }
}

#[tauri::command]
fn move_project(ctx: State<'_, Ctx>, root: String, to: usize) {
    if ctx.store.lock().unwrap().move_project(&root, to) {
        server::publish(&ctx);
    }
}

#[tauri::command]
fn set_shelved(ctx: State<'_, Ctx>, root: String, shelved: bool) {
    if ctx.store.lock().unwrap().set_shelved(&root, shelved) {
        server::publish(&ctx);
    }
}

#[tauri::command]
fn close_project(ctx: State<'_, Ctx>, root: String) -> Result<(), String> {
    ctx.store.lock().unwrap().close_project(&root)?;
    server::publish(&ctx);
    Ok(())
}

#[tauri::command]
fn dismiss_alert(ctx: State<'_, Ctx>, session_id: String, alert_id: u64) {
    ctx.store
        .lock()
        .unwrap()
        .dismiss_alert(&session_id, alert_id);
    server::publish(&ctx);
}

#[tauri::command]
fn dismiss_session(ctx: State<'_, Ctx>, session_id: String) {
    if ctx.store.lock().unwrap().dismiss_session(&session_id) {
        server::publish(&ctx);
    }
}

/// Card click: focus the session's Windows Terminal tab, or resume it if closed.
#[cfg(windows)]
#[tauri::command]
async fn activate_session(
    ctx: State<'_, Ctx>,
    session_id: String,
) -> Result<focus::Outcome, String> {
    let session = ctx
        .store
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
        .ok_or("unknown session")?;
    let cfg = ctx.config.read().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || focus::activate(&session, &cfg))
        .await
        .map_err(|e| e.to_string())
}

/// Global hotkey: focus the session that has been waiting on you the longest.
#[cfg(windows)]
fn jump_to_waiting(ctx: &Ctx) {
    let target = {
        let store = ctx.store.lock().unwrap();
        store
            .ordered()
            .into_iter()
            .filter(|s| {
                matches!(
                    s.status,
                    session::Status::NeedsPermission | session::Status::NeedsInput
                )
            })
            .min_by_key(|s| s.status_since)
            .cloned()
    };
    if let Some(s) = target {
        let cfg = ctx.config.read().unwrap().clone();
        tauri::async_runtime::spawn_blocking(move || focus::activate(&s, &cfg));
    }
}

/// (Re)register the jump hotkey from config.
#[cfg(windows)]
fn register_hotkey(app: &tauri::AppHandle, ctx: &Ctx) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let key = ctx.config.read().unwrap().jump_hotkey.clone();
    if key.is_empty() {
        return;
    }
    let ctx = ctx.clone();
    if let Err(e) = gs.on_shortcut(key.as_str(), move |_, _, ev| {
        if ev.state == ShortcutState::Pressed {
            jump_to_waiting(&ctx);
        }
    }) {
        log::warn!("hotkey {key}: {e}");
    }
}

#[tauri::command]
fn answer_permission(ctx: State<'_, Ctx>, session_id: String, allow: bool) -> Result<(), String> {
    server::answer_permission(&ctx, &session_id, allow)
}

/// end | folder | vscode. No "interrupt": SIGINT ends claude too, and stopping a turn needs a
/// keypress in its terminal, which we don't inject.
#[cfg(windows)]
#[tauri::command]
async fn session_action(
    ctx: State<'_, Ctx>,
    session_id: String,
    action: String,
) -> Result<(), String> {
    let s = ctx
        .store
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
        .ok_or("unknown session")?;
    let cfg = ctx.config.read().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "end" => actions::signal(&s, &cfg, "TERM"),
        "folder" => actions::open_folder(&s, &cfg),
        "vscode" => actions::open_vscode(&s, &cfg),
        other => Err(format!("unknown action {other}")),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// folder | vscode for a tracked project's root.
#[cfg(windows)]
#[tauri::command]
fn project_action(ctx: State<'_, Ctx>, root: String, action: String) -> Result<(), String> {
    let distro = ctx.config.read().unwrap().distro.clone();
    match action.as_str() {
        "folder" => actions::open_folder_at(&distro, &root),
        "vscode" => actions::open_vscode_at(&distro, &root),
        other => Err(format!("unknown action {other}")),
    }
}

#[cfg(windows)]
#[tauri::command]
async fn recent_projects(ctx: State<'_, Ctx>) -> Result<Vec<actions::Recent>, String> {
    let sessions = ctx.store.lock().unwrap().snapshot();
    let (cfg, overrides) = (
        ctx.config.read().unwrap().clone(),
        ctx.overrides.read().unwrap().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        actions::recent_projects(&sessions, &cfg, &overrides, 15)
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
#[tauri::command]
async fn new_session(ctx: State<'_, Ctx>, path: String) -> Result<(), String> {
    let (cfg, overrides) = (
        ctx.config.read().unwrap().clone(),
        ctx.overrides.read().unwrap().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || actions::new_session(&path, &cfg, &overrides))
        .await
        .map_err(|e| e.to_string())?
}

/// "Add project": track a folder's project with no session yet (SPEC §3.12).
#[cfg(windows)]
#[tauri::command]
fn track_project(ctx: State<'_, Ctx>, path: String) -> Result<String, String> {
    let (cfg, overrides) = (
        ctx.config.read().unwrap().clone(),
        ctx.overrides.read().unwrap().clone(),
    );
    let p = actions::project_at(&path, &cfg, &overrides)?;
    let name = p.name.clone();
    ctx.store.lock().unwrap().track(p, server::now_ms());
    server::publish(&ctx);
    Ok(name)
}

/// Windows folder picker, returned as a WSL path (None if cancelled or not mappable).
#[cfg(windows)]
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle, ctx: State<'_, Ctx>) -> Result<Option<String>, ()> {
    use tauri_plugin_dialog::DialogExt;
    let distro = ctx.config.read().unwrap().distro.clone();
    let picked = app
        .dialog()
        .file()
        .set_title("Start a Claude Code session in…")
        .set_directory(format!(r"\\wsl.localhost\{distro}\home"))
        .blocking_pick_folder();
    Ok(picked.and_then(|p| actions::linux_path(&p.to_string())))
}

#[tauri::command]
fn set_collapsed(app: tauri::AppHandle, ctx: State<'_, Ctx>, collapsed: bool) {
    peek::hide(&app);
    server::set_collapsed(&ctx, collapsed);
}

/// mode = "docked" | "floating"; monitor = 0 (primary) or n. Written to config.toml, which the
/// watcher then applies.
#[tauri::command]
fn set_window(mode: Option<String>, monitor: Option<usize>) -> Result<(), String> {
    if let Some(m) = mode {
        config::set_value("mode", &format!("\"{m}\""))?;
    }
    if let Some(n) = monitor {
        config::set_value("monitor", &n.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
fn list_monitors() -> Vec<appbar::Monitor> {
    appbar::monitors()
}

#[tauri::command]
fn open_settings_file(app: tauri::AppHandle) {
    let _ = config::Config::load(); // creates the file if it doesn't exist yet
    if let Some(p) = config::path() {
        use tauri_plugin_opener::OpenerExt;
        let _ = app.opener().open_path(p.to_string_lossy(), None::<&str>);
    }
}

/// The settings window (`index.html?view=settings`), created on first use. Async: on Windows a
/// window built from a synchronous command can deadlock.
#[tauri::command]
async fn open_settings(app: tauri::AppHandle, tab: Option<String>) -> Result<(), String> {
    show_settings(&app, tab.as_deref()).map_err(|e| e.to_string())
}

/// Show the settings window, optionally on one tab ("general", "alerts", "projects", …).
fn show_settings(app: &tauri::AppHandle, tab: Option<&str>) -> tauri::Result<()> {
    use tauri::Emitter;
    if let Some(w) = app.get_webview_window("settings") {
        if let Some(t) = tab {
            let _ = app.emit_to("settings", "settings-tab", t);
        }
        let _ = w.unminimize();
        return w.set_focus();
    }
    let url = match tab {
        Some(t) => format!("index.html?view=settings&tab={t}"),
        None => "index.html?view=settings".into(),
    };
    tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App(url.into()))
        .title("cc-sidebar settings")
        .inner_size(680.0, 640.0)
        .min_inner_size(560.0, 420.0)
        .center()
        .build()?;
    Ok(())
}

#[tauri::command]
fn get_settings(ctx: State<'_, Ctx>) -> settings::Settings {
    let roots: Vec<String> = ctx
        .store
        .lock()
        .unwrap()
        .projects()
        .iter()
        .map(|t| t.project.root.clone())
        .collect();
    settings::load(&roots)
}

/// Write the settings window's changes; the file watcher applies them.
#[tauri::command]
fn save_settings(config: config::Config, projects: Vec<project::Override>) -> Result<(), String> {
    #[cfg(windows)]
    if !config.jump_hotkey.is_empty() {
        use std::str::FromStr;
        tauri_plugin_global_shortcut::Shortcut::from_str(&config.jump_hotkey)
            .map_err(|e| format!("Hotkey \"{}\": {e}", config.jump_hotkey))?;
    }
    settings::validate(&config)?;
    settings::validate_projects(&projects)?;
    settings::save_projects(&projects)?;
    settings::save_config(&config)
}

/// A .wav file for an alert sound.
#[tauri::command]
async fn pick_sound(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("Alert sound")
        .add_filter("Sounds", &["wav"])
        .set_directory(r"C:\Windows\Media")
        .blocking_pick_file()
        .map(|p| p.to_string())
}

#[cfg(windows)]
#[tauri::command]
fn list_sounds() -> alerts::SoundChoices {
    alerts::sound_choices()
}

#[cfg(windows)]
#[tauri::command]
fn play_sound(sound: String) {
    alerts::play(&sound);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store: Shared = Arc::new(Mutex::new(store::Store::default()));
    server::load_saved(&mut store.lock().unwrap());
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_collapsed,
            set_window,
            open_settings_file,
            open_settings,
            get_settings,
            save_settings,
            pick_sound,
            #[cfg(windows)]
            play_sound,
            #[cfg(windows)]
            list_sounds,
            #[cfg(windows)]
            list_monitors,
            list_tasks,
            open_projects_file,
            #[cfg(windows)]
            start_task,
            toggle_star,
            peek_show,
            peek_resize,
            peek_hide,
            rename_session,
            move_project,
            set_shelved,
            close_project,
            dismiss_alert,
            dismiss_session,
            answer_permission,
            #[cfg(windows)]
            activate_session,
            #[cfg(windows)]
            session_action,
            #[cfg(windows)]
            project_action,
            #[cfg(windows)]
            recent_projects,
            #[cfg(windows)]
            new_session,
            #[cfg(windows)]
            track_project,
            #[cfg(windows)]
            pick_folder
        ])
        .setup(move |app| {
            let window = app.get_webview_window("main").expect("main window");
            let hwnd = window.hwnd()?.0 as isize;
            let cfg = config::Config::load();
            let prefs = ui::UiPrefs::load();
            #[cfg(windows)]
            appbar::attach(
                windows::Win32::Foundation::HWND(hwnd as *mut _),
                server::placement(&cfg, &prefs),
            );
            let ctx = Ctx {
                store,
                app: app.handle().clone(),
                overrides: Arc::new(RwLock::new(project::Overrides::load())),
                config: Arc::new(RwLock::new(cfg)),
                hwnd,
                notifier: Arc::new(Mutex::new(alerts::Notifier::default())),
                recapper: recap::Recapper::default(),
                holds: Default::default(),
                prefs: Arc::new(Mutex::new(prefs)),
                usage: Default::default(),
            };
            // Restored alerts are old news: mark them seen without a sound.
            ctx.notifier
                .lock()
                .unwrap()
                .scan(&ctx.store.lock().unwrap(), true);
            #[cfg(windows)]
            peek::init(app.handle());
            server::apply_config(&ctx);
            server::refresh_all_projects(&ctx);
            server::backfill_names(&ctx);
            #[cfg(windows)]
            {
                register_hotkey(app.handle(), &ctx);
                let (handle, c) = (app.handle().clone(), ctx.clone());
                tauri::Listener::listen(app, "config-reloaded", move |_| {
                    register_hotkey(&handle, &c)
                });
            }
            app.manage(ctx.clone());

            let collapse =
                MenuItem::with_id(app, "collapse", "Collapse / expand", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit cc-sidebar", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("app icon"))
                .tooltip("cc-sidebar")
                .menu(&Menu::with_items(app, &[&collapse, &settings, &quit])?)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "collapse" => {
                        let ctx = app.state::<Ctx>();
                        let collapsed = ctx.prefs.lock().unwrap().collapsed;
                        server::set_collapsed(&ctx, !collapsed);
                    }
                    "settings" => {
                        let _ = show_settings(app, None);
                    }
                    "quit" => {
                        // Give the reserved space back first, so desktop icons can be restored.
                        #[cfg(windows)]
                        appbar::release(windows::Win32::Foundation::HWND(
                            app.state::<Ctx>().hwnd as *mut _,
                        ));
                        app.exit(0)
                    }
                    _ => {}
                })
                .build(app)?;

            tauri::async_runtime::spawn(server::run(ctx.clone()));
            tauri::async_runtime::spawn(server::ticker(ctx.clone()));
            tauri::async_runtime::spawn(server::watch_config(ctx.clone()));
            tauri::async_runtime::spawn(server::probe_usage(ctx.clone()));
            tauri::async_runtime::spawn(liveness::run(ctx));
            Ok(())
        })
        // Closed by Windows or another program (e.g. `taskkill`, an installer): give the
        // reserved space back and restore desktop icons first, like Quit in the tray.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event
                && window.label() == "main"
            {
                #[cfg(windows)]
                appbar::release(windows::Win32::Foundation::HWND(
                    window.state::<Ctx>().hwnd as *mut _,
                ));
                window.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
