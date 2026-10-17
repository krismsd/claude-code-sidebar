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
