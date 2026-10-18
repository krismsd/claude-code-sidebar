//! Card actions on Windows Terminal (SPEC §3.6): focus a session's tab via UI Automation, or
//! resume a closed session in a new tab.
use crate::config::Config;
use crate::session::{Session, Status};
use serde::Serialize;
use uiautomation::UIAutomation;
use uiautomation::patterns::UISelectionItemPattern;
use uiautomation::types::ControlType;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Focused,
    Resumed,
    /// Open session but no matching tab (e.g. running in VS Code or tmux).
    TabNotFound,
    /// Started with plain `claude` (no tag), and no tab title gave it away.
    Untagged,
    Failed,
}

/// Click on a card: closed → resume in a new tab; open → focus its tab.
pub fn activate(s: &Session, cfg: &Config) -> Outcome {
    if s.status == Status::Closed {
        return match resume(s, cfg) {
            Ok(()) => Outcome::Resumed,
            Err(e) => {
                log::warn!("resume {}: {e}", s.session_id);
                Outcome::Failed
            }
        };
    }
    // Tagged sessions (started with `claudes`) have "[cc:<tag>]" in the tab title; plain `claude`
    // sessions fall back to the project name.
    let needle = match (&s.tag, &s.project) {
        (Some(tag), _) => format!("[cc:{tag}]"),
        // Plain `claude` sets its own task-summary titles, so this rarely matches.
        (None, Some(p)) => p.name.clone(),
        (None, None) => return Outcome::Untagged,
    };
    match focus_tab(&needle) {
        Ok(true) => Outcome::Focused,
        Ok(false) if s.tag.is_none() => Outcome::Untagged,
        Ok(false) => Outcome::TabNotFound,
        Err(e) => {
            log::warn!("focus {needle}: {e}");
            Outcome::Failed
        }
    }
}

/// Select the first WT tab whose name contains `needle` and bring its window forward.
fn focus_tab(needle: &str) -> Result<bool, uiautomation::Error> {
    let ua = UIAutomation::new()?;
    let root = ua.get_root_element()?;
    let windows = ua
        .create_matcher()
        .from(root)
        .depth(2)
        .classname("CASCADIA_HOSTING_WINDOW_CLASS")
        .timeout(0)
        .find_all()
        .unwrap_or_default();
    for w in windows {
        let tabs = ua
            .create_matcher()
            .from(w.clone())
            .control_type(ControlType::TabItem)
            .timeout(0)
            .find_all()
            .unwrap_or_default();
        for tab in tabs {
            if !tab.get_name().unwrap_or_default().contains(needle) {
                continue;
            }
            // Selecting a tab moves keyboard focus to the tab strip, so only select it if it
            // isn't already the active tab, and then hand focus back to its terminal.
            let item = tab.get_pattern::<UISelectionItemPattern>()?;
            let switched = !item.is_selected().unwrap_or(false);
            if switched {
                item.select()?;
            }
            let hwnd: isize = w.get_native_window_handle()?.into();
            bring_forward(hwnd);
            if switched {
                focus_terminal(&ua, &w);
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Give keyboard focus to the visible terminal pane (`TermControl`) of a WT window. The new
/// tab's pane can take a moment to appear after switching, so retry briefly.
fn focus_terminal(ua: &UIAutomation, window: &uiautomation::UIElement) {
    for _ in 0..10 {
        let panes = ua
            .create_matcher()
            .from(window.clone())
            .classname("TermControl")
            .timeout(0)
            .find_all()
            .unwrap_or_default();
        if let Some(p) = panes
            .into_iter()
            .find(|p| !p.is_offscreen().unwrap_or(true))
        {
            let _ = p.set_focus();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(windows)]
fn bring_forward(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        // We are the foreground process (the user just clicked us), so this is allowed.
        let _ = SetForegroundWindow(h);
    }
}

fn resume(s: &Session, cfg: &Config) -> Result<(), String> {
    let cwd = s.cwd.as_deref().ok_or("no cwd")?;
    let distro = s.distro.as_deref().unwrap_or(&cfg.distro);
    let name = s
        .project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("claude");
    launch(
        cfg,
        distro,
        cwd,
        name,
        &format!("--resume {}", s.session_id),
        &[],
    )
}

/// Open a new WT tab running the launcher (the `claudes` wrapper) in `cwd`:
/// `wt.exe -w <window> new-tab … wsl.exe -d <distro> --cd <cwd> -e bash -lc "env … <launcher> <args> || true"`.
/// wt.exe splits its command line on `;`, so nothing we pass may contain one.
pub fn launch(
    cfg: &Config,
    distro: &str,
    cwd: &str,
    name: &str,
    args: &str,
    extra_env: &[(&str, &str)],
) -> Result<(), String> {
    let env: String = cfg
        .launch_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .chain(extra_env.iter().copied())
        .map(|(k, v)| format!("{k}={v} "))
        .collect();
    // `|| true`: Claude Code is the tab's only process, and Windows Terminal only closes a tab
    // by itself on a clean exit, so a session ended from the sidebar (SIGTERM, exit 143) would
    // leave a dead tab. (No `;` allowed: wt.exe splits on it.)
    let cmd = format!("env {env}{} {args} || true", cfg.launcher);
    let title = format!("[cc] {name}");
    let argv = [
        "-w",
        &cfg.terminal_window,
        "new-tab",
        "--title",
        &title,
        "wsl.exe",
        "-d",
        distro,
        "--cd",
        cwd,
        "-e",
        "bash",
        "-lc",
        &cmd,
    ];
    if argv.iter().any(|a| a.contains(';')) {
        return Err("argument contains ';'".into());
    }
    std::process::Command::new("wt.exe")
        .args(argv)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
