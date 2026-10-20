//! Alert side effects (SPEC §3.3, M4): a sound per alert kind and a Windows toast when the
//! sidebar can't be seen. Detects alerts (and new permission prompts) not seen before.
use crate::config::Config;
use crate::session::{AlertKind, Ms, Status};
use crate::store::Store;
use std::collections::HashSet;

/// Something new to announce.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    /// Config key for the sound: the alert kind in snake_case, or "permission".
    pub sound_key: &'static str,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct Notifier {
    seen_alerts: HashSet<(String, u64)>,
    seen_permissions: HashSet<(String, Ms)>,
}

fn kind_key(k: AlertKind) -> &'static str {
    match k {
        AlertKind::Denied => "denied",
        AlertKind::StopFailure => "stop_failure",
        AlertKind::RepeatedFailure => "repeated_failure",
        AlertKind::Waiting => "waiting",
        AlertKind::Slow => "slow",
        AlertKind::Compact => "compact",
        AlertKind::Finished => "finished",
    }
}

impl Notifier {
    /// Everything not yet seen; marks it seen. Call with `quiet` at startup so restored alerts
    /// don't all go off at once.
    pub fn scan(&mut self, store: &Store, quiet: bool) -> Vec<Announcement> {
        let mut out = Vec::new();
        for s in store.ordered() {
            let name = s
                .project
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Claude Code".into());
            for a in &s.alerts {
                if self.seen_alerts.insert((s.session_id.clone(), a.id)) && !quiet {
                    out.push(Announcement {
                        sound_key: kind_key(a.kind),
                        title: name.clone(),
                        body: a.text.clone(),
                    });
                }
            }
            if s.status == Status::NeedsPermission
                && let Some(p) = &s.pending_permission
                && self
                    .seen_permissions
                    .insert((s.session_id.clone(), p.since))
                && !quiet
            {
                out.push(Announcement {
                    sound_key: "permission",
                    title: name.clone(),
                    body: format!("Needs permission: {}", p.summary),
                });
            }
        }
        out
    }
}

/// Play the configured sound: a system sound alias or a .wav path; "" = silent.
#[cfg(windows)]
pub fn play(sound: &str) {
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ALIAS, SND_ASYNC, SND_FILENAME, SND_NODEFAULT,
    };
    use windows::core::HSTRING;
    if sound.is_empty() {
        return;
    }
    let kind = if sound.to_ascii_lowercase().ends_with(".wav") {
        SND_FILENAME
    } else {
        SND_ALIAS
    };
    unsafe {
        let _ = PlaySoundW(
            &HSTRING::from(sound),
            None,
            kind | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Sounds to choose from in settings: the files in Windows' Media folder, and the sound scheme
/// aliases with the file each plays (on Windows 11 most aliases share one file).
#[derive(Debug, serde::Serialize)]
pub struct SoundChoices {
    pub files: Vec<SoundChoice>,
    pub aliases: Vec<SoundChoice>,
}

#[derive(Debug, serde::Serialize)]
pub struct SoundChoice {
    /// What goes in config.toml: a .wav path or an alias.
    pub value: String,
    pub label: String,
    /// For an alias: the file it plays now ("" = nothing).
    pub plays: Option<String>,
}

pub const ALIASES: [(&str, &str); 6] = [
    ("SystemAsterisk", "Asterisk"),
    ("SystemExclamation", "Exclamation"),
    ("SystemHand", "Critical stop"),
    ("SystemNotification", "Notification"),
    ("SystemQuestion", "Question"),
    ("SystemDefault", "Default beep"),
];

/// "Windows Notify Email.wav" → "Notify Email"; "tada.wav" → "Tada".
fn sound_label(file: &str) -> String {
    let stem = file.strip_suffix(".wav").unwrap_or(file);
    let stem = stem.strip_prefix("Windows ").unwrap_or(stem);
    let mut c = stem.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn sound_choices() -> SoundChoices {
    let media = std::path::PathBuf::from(
        std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()),
    )
    .join("Media");
    let mut files: Vec<SoundChoice> = std::fs::read_dir(&media)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    // Alarms and ringtones are long loops, not alert sounds.
                    let long = name.starts_with("Alarm") || name.starts_with("Ring");
                    (name.to_ascii_lowercase().ends_with(".wav") && !long).then(|| SoundChoice {
                        value: e.path().to_string_lossy().into_owned(),
                        label: sound_label(&name),
                        plays: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|f| f.label.to_lowercase());
    let aliases = ALIASES
        .iter()
        .map(|(alias, label)| SoundChoice {
            value: alias.to_string(),
            label: label.to_string(),
            plays: Some(
                scheme_file(alias)
                    .map(|f| sound_label(&f))
                    .unwrap_or_default(),
            ),
        })
        .collect();
    SoundChoices { files, aliases }
}

/// The file an alias plays in the current sound scheme (None = silent).
#[cfg(windows)]
fn scheme_file(alias: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::HSTRING;
    // SystemDefault is ".Default" in the registry.
    let key = if alias == "SystemDefault" {
        ".Default"
    } else {
        alias
    };
    let sub = HSTRING::from(format!(r"AppEvents\Schemes\Apps\.Default\{key}\.Current"));
    let mut buf = vec![0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &sub,
            None,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
    };
    if ok.is_err() {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
    let name = path.rsplit('\\').next()?.to_string();
    (!name.is_empty()).then_some(name)
}

/// Whether to show a toast for this config, given whether the sidebar is currently hidden.
pub fn wants_toast(cfg: &Config, sidebar_hidden: bool) -> bool {
    match cfg.toasts.as_str() {
        "always" => true,
        "never" => false,
        _ => sidebar_hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Hook, HookMeta};

    fn hook(v: serde_json::Value) -> Hook {
        Hook::parse(v.to_string().as_bytes(), HookMeta::default()).unwrap()
    }

    #[test]
    fn announces_each_alert_and_permission_once() {
        let mut st = Store::default();
        let mut n = Notifier::default();
        st.apply(&hook(serde_json::json!({"session_id": "a", "hook_event_name": "PreCompact", "trigger": "auto"})), 1);
        assert!(n.scan(&st, true).is_empty(), "quiet start");
        st.apply(
            &hook(serde_json::json!({"session_id": "a", "hook_event_name": "PermissionDenied",
                "tool_name": "Bash", "tool_input": {"command": "rm -rf x"}, "reason": "[Destructive]"})),
            2,
        );
        st.apply(
            &hook(
                serde_json::json!({"session_id": "a", "hook_event_name": "PermissionRequest",
                "tool_name": "Bash", "tool_input": {"command": "touch y"}}),
            ),
            3,
        );
        let got = n.scan(&st, false);
        assert_eq!(
            got.iter().map(|a| a.sound_key).collect::<Vec<_>>(),
            ["denied", "permission"]
        );
        assert!(got[0].body.contains("[Destructive]"));
        assert!(n.scan(&st, false).is_empty(), "only once");
    }

    #[test]
    fn sound_labels() {
        assert_eq!(sound_label("Windows Notify Email.wav"), "Notify Email");
        assert_eq!(sound_label("tada.wav"), "Tada");
    }

    #[test]
    fn toast_policy() {
        let mut c = Config::default();
        assert!(!wants_toast(&c, false));
        assert!(wants_toast(&c, true));
        c.toasts = "always".into();
        assert!(wants_toast(&c, false));
        c.toasts = "never".into();
        assert!(!wants_toast(&c, true));
    }
}
