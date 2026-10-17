//! Small UI state that isn't configuration: collapsed or not, and where the floating window
//! was left. Saved in `%APPDATA%\\cc-sidebar\\ui.json`.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    #[serde(default)]
    pub collapsed: bool,
    /// Floating window x, y, width, height in physical pixels.
    #[serde(default)]
    pub float_rect: Option<(i32, i32, i32, i32)>,
}

fn path() -> Option<std::path::PathBuf> {
    crate::config::dir().map(|d| d.join("ui.json"))
}

impl UiPrefs {
    pub fn load() -> UiPrefs {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let (Some(p), Ok(json)) = (path(), serde_json::to_string_pretty(self)) {
            let _ = std::fs::write(p, json);
        }
    }
}
