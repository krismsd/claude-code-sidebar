//! `%APPDATA%\cc-sidebar\config.toml` (SPEC §3.10). Created with defaults on first run, and
//! hot-reloaded: the app polls its modified time.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "docked" (an AppBar that reserves its screen space) or "floating" (a normal window).
    pub mode: String,
    /// Monitor for the sidebar: 0 = primary, n = the nth counting from the left.
    pub monitor: usize,
    /// "right" or "left".
    pub edge: String,
    /// Sidebar width in device-independent pixels.
    pub width: i32,
    /// WSL distro for sessions that don't report one.
    pub distro: String,
    /// Command (inside WSL) that starts Claude Code for resume and new sessions.
    pub launcher: String,
    /// Extra environment for launched sessions, e.g. CC_SIDEBAR_SETTINGS for test hooks.
    pub launch_env: BTreeMap<String, String>,
    /// Windows Terminal window for new/resumed tabs (`wt.exe -w`): "0" = the last used one,
    /// "new", or a window name.
    pub terminal_window: String,
    pub waiting_minutes: u64,
    pub slow_minutes: u64,
    pub closed_retention_days: u64,
    /// Sound per alert kind (and `permission` for new permission prompts): a Windows system
    /// sound alias such as "SystemExclamation", a path to a .wav file, or "" for silence.
    pub sounds: BTreeMap<String, String>,
    /// Windows toast notifications for alerts: "when_hidden" (a full-screen app covers the
    /// sidebar), "always" or "never".
    pub toasts: String,
    /// Global hotkey that focuses the session waiting on you the longest ("" to disable).
    pub jump_hotkey: String,
    /// Haiku recaps after each turn (SPEC §3.5): command inside WSL ("" disables them).
    pub recap_command: String,
    /// Minimum seconds between Haiku recaps of one session.
    pub recap_min_interval_s: u64,
    /// Fetches usage limits once at startup (one tiny Haiku call), run inside WSL ("" = off;
    /// they still update with each recap).
    pub usage_command: String,
    /// Lists a project's tasks via Claude + a claude.ai connector (SPEC §3.12), run inside WSL.
    pub tasks_command: String,
    /// Start cc-sidebar when you sign in to Windows (installed builds only).
    pub autostart: bool,
    /// Where sessions run: "terminal" (in a Windows Terminal tab, as plain `claude` would),
    /// "tmux" (in tmux, shown in Windows Terminal: closing the tab keeps it running) or
    /// "window" (in tmux, shown in the sidebar's own terminal window).
    pub session_mode: String,
}

fn default_sounds() -> BTreeMap<String, String> {
    [
        // Files, not scheme aliases: on Windows 11 most aliases play the same sound.
        ("denied", r"C:\Windows\Media\Windows Exclamation.wav"),
        (
            "stop_failure",
            r"C:\Windows\Media\Windows Critical Stop.wav",
        ),
        ("repeated_failure", r"C:\Windows\Media\Windows Error.wav"),
        (
            "waiting",
            r"C:\Windows\Media\Windows Notify System Generic.wav",
        ),
        (
            "finished",
            r"C:\Windows\Media\Windows Notify System Generic.wav",
        ),
        ("slow", ""),
        ("compact", ""),
        ("permission", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: "docked".into(),
            monitor: 0,
            edge: "right".into(),
            width: 320,
            distro: "Ubuntu".into(),
            launcher: "claudes".into(),
            launch_env: BTreeMap::new(),
            terminal_window: "0".into(),
            waiting_minutes: 2,
            slow_minutes: 10,
            closed_retention_days: 7,
            sounds: default_sounds(),
            toasts: "when_hidden".into(),
            jump_hotkey: "Ctrl+Alt+J".into(),
            recap_command: "cc-recap".into(),
            recap_min_interval_s: 30,
            usage_command: "cc-usage".into(),
            tasks_command: "cc-tasks".into(),
            autostart: false,
            session_mode: "terminal".into(),
        }
    }
}

pub const TEMPLATE: &str = r#"# cc-sidebar settings. Saved changes apply within a couple of seconds.

mode = "docked"           # "docked" (reserves its screen space) or "floating" (a normal window)
monitor = 0               # 0 = primary, 1, 2, … = monitors counting from the left
edge = "right"            # "right" or "left"
width = 320               # sidebar width (DIP)
distro = "Ubuntu"         # WSL distro for sessions that don't report one
launcher = "claudes"      # command inside WSL used to resume / start sessions
terminal_window = "0"     # WT window for new/resumed tabs: "0" = last used, "new", or a name
waiting_minutes = 2       # ⏳ alert when a session waits on you this long
slow_minutes = 10         # 🐌 alert when a working session is silent this long
closed_retention_days = 7 # closed sessions are pruned after this many days
toasts = "when_hidden"    # Windows toasts for alerts: "when_hidden" (full-screen app), "always", "never"
jump_hotkey = "Ctrl+Alt+J" # focus the session waiting on you longest ("" to disable)
recap_command = "cc-recap" # Haiku recap after each turn, run inside WSL ("" to disable)
recap_min_interval_s = 30  # at most one Haiku recap per session this often
usage_command = "cc-usage" # usage limits at startup: one tiny Haiku call ("" to skip)
tasks_command = "cc-tasks" # lists project tasks via Claude + connectors (see projects.toml)
autostart = false          # start cc-sidebar when you sign in to Windows
session_mode = "terminal"  # "terminal", "tmux" (keeps running when the tab closes) or "window" (tmux, in the sidebar's own window)

# Sound per alert: a .wav path, a Windows sound alias (e.g. "SystemHand"), or "" for none.
[sounds]
denied = 'C:\Windows\Media\Windows Exclamation.wav'      # 🚫 auto-mode denial
stop_failure = 'C:\Windows\Media\Windows Critical Stop.wav' # ⚠️ API error
repeated_failure = 'C:\Windows\Media\Windows Error.wav'  # 🔁 same tool failing
waiting = 'C:\Windows\Media\Windows Notify System Generic.wav' # ⏳ waiting on you
finished = 'C:\Windows\Media\Windows Notify System Generic.wav' # ✅ background job done, ready for you
slow = ""                          # 🐌 silent while working
compact = ""                       # 📦 compacting context
permission = ""                    # a new permission prompt

# Extra environment for sessions the sidebar launches.
[launch_env]
"#;

pub fn dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("cc-sidebar"))
}

pub fn path() -> Option<PathBuf> {
    dir().map(|d| d.join("config.toml"))
}

impl Config {
    /// Read the config file, creating it from the template if missing. Errors fall back to
    /// defaults (logged) so a typo never stops the sidebar.
    pub fn load() -> Config {
        let Some(path) = path() else {
            return Config::default();
        };
        if !path.exists() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, TEMPLATE);
        }
        match std::fs::read_to_string(&path).map(|t| toml::from_str::<Config>(&t)) {
            Ok(Ok(c)) => c.with_default_sounds(),
            Ok(Err(e)) => {
                log::warn!("{}: {e}", path.display());
                Config::default()
            }
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                Config::default()
            }
        }
    }

    /// A `[sounds]` table only needs to list what it changes.
    fn with_default_sounds(mut self) -> Config {
        for (k, v) in default_sounds() {
            self.sounds.entry(k).or_insert(v);
        }
        self
    }

    /// Sessions run in tmux (they survive their terminal being closed).
    pub fn tmux(&self) -> bool {
        matches!(self.session_mode.as_str(), "tmux" | "window")
    }

    /// Sessions are shown in the sidebar's own terminal window.
    pub fn viewer(&self) -> bool {
        self.session_mode == "window"
    }

    pub fn floating(&self) -> bool {
        self.mode.eq_ignore_ascii_case("floating")
    }

    pub fn modified() -> Option<std::time::SystemTime> {
        std::fs::metadata(path()?).and_then(|m| m.modified()).ok()
    }
}

/// Set one top-level `key = value` in config.toml (value as TOML, e.g. `"floating"` or `2`),
/// keeping comments and everything else. Used by the sidebar's ⚙ menu; the file watcher then
/// applies it.
pub fn set_value(key: &str, value: &str) -> Result<(), String> {
    let path = path().ok_or("no APPDATA")?;
    let _ = Config::load(); // creates the file if missing
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    std::fs::write(&path, with_value(&text, key, value)).map_err(|e| e.to_string())
}

/// `text` with `key = value` replaced (keeping its trailing comment) or added before the first
/// table.
pub fn with_value(text: &str, key: &str, value: &str) -> String {
    let mut out = Vec::new();
    let mut done = false;
    let mut in_table = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            if !done {
                out.push(format!("{key} = {value}"));
                done = true;
            }
            in_table = true;
        }
        let is_key = !in_table
            && t.split_once('=').is_some_and(|(k, _)| k.trim() == key)
            && !t.starts_with('#');
        if is_key && !done {
            let comment = line.find(" #").map(|i| &line[i..]).unwrap_or("");
            out.push(format!("{key} = {value}{comment}"));
            done = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !done {
        out.push(format!("{key} = {value}"));
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(
            toml::from_str::<Config>(TEMPLATE).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn set_value_keeps_comments() {
        let t = with_value(TEMPLATE, "mode", "\"floating\"");
        assert!(t.contains("mode = \"floating\" # \"docked\""), "{t}");
        let c: Config = toml::from_str(&t).unwrap();
        assert!(c.floating());
        let t = with_value("width = 300\n[sounds]\ndenied = \"\"\n", "monitor", "2");
        assert_eq!(t, "width = 300\nmonitor = 2\n[sounds]\ndenied = \"\"\n");
        assert_eq!(toml::from_str::<Config>(&t).unwrap().monitor, 2);
    }

    #[test]
    fn partial_files_keep_defaults() {
        let c: Config =
            toml::from_str("width = 400\n[launch_env]\nCC_SIDEBAR_SETTINGS = \"/x.json\"\n")
                .unwrap();
        assert_eq!(c.width, 400);
        assert_eq!(c.edge, "right");
        assert_eq!(c.launch_env["CC_SIDEBAR_SETTINGS"], "/x.json");
        let c: Config = toml::from_str::<Config>("[sounds]\ndenied = \"\"\n")
            .unwrap()
            .with_default_sounds();
        assert_eq!(c.sounds["denied"], "");
        assert!(c.sounds["stop_failure"].ends_with("Critical Stop.wav"));
    }
}
