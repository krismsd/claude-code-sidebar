//! Is the user at the computer? The "ready" pill fades over the time a session has been in
//! front of the user and left alone, not over lunch or a night away. Windows reports when the
//! keyboard or mouse were last used (`GetLastInputInfo`, which also covers a locked screen and
//! sleep); no input for `AWAY_AFTER` makes an away spell, counted from that last input. Spells
//! are saved (`presence.json`), and time the app wasn't running counts as away too.
use crate::server::{Ctx, now_ms, publish};
use crate::session::Ms;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// No input for this long means away (from the last input).
pub const AWAY_AFTER: Ms = 5 * 60_000;
/// Spells older than this are forgotten (nothing ready fades for longer).
const KEEP: Ms = 7 * 24 * 3_600_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    /// Away spells: (from, until); `until` is None while still away.
    pub away: Vec<(Ms, Option<Ms>)>,
    /// When the user was last seen using the computer.
    pub last_input: Ms,
}

impl Presence {
    /// The latest input was at `last_input` (it's `now` or earlier). Returns whether the
    /// spells changed.
    pub fn observe(&mut self, last_input: Ms, now: Ms) -> bool {
        self.last_input = self.last_input.max(last_input);
        let away = now.saturating_sub(self.last_input) >= AWAY_AFTER;
        let open = self.away.last().is_some_and(|(_, until)| until.is_none());
        let changed = match (away, open) {
            (true, false) => {
                self.away.push((self.last_input, None));
                true
            }
            (false, true) => {
                if let Some(last) = self.away.last_mut() {
                    last.1 = Some(self.last_input);
                }
                true
            }
            _ => false,
        };
        self.away
            .retain(|(_, until)| until.is_none_or(|u| now.saturating_sub(u) < KEEP));
        changed
    }

    /// Starting up: the time since the user was last seen (the app or the computer was off)
    /// was away, if it was long enough.
    pub fn resume(&mut self, now: Ms) {
        if self.last_input > 0 {
            self.observe(self.last_input, now);
        }
    }
}

fn path() -> Option<std::path::PathBuf> {
    crate::config::dir().map(|d| d.join("presence.json"))
}

pub fn load() -> Presence {
    let mut p: Presence = path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    p.resume(now_ms());
    p
}

fn save(p: &Presence) {
    if let (Some(path), Ok(json)) = (path(), serde_json::to_string(p)) {
        let _ = std::fs::write(path, json);
    }
}

/// When the keyboard or mouse were last used, as epoch ms.
#[cfg(windows)]
fn last_input(now: Ms) -> Option<Ms> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return None;
        }
        // Both are 32-bit tick counts (they wrap together every 49 days).
        let idle = GetTickCount().wrapping_sub(info.dwTime) as Ms;
        Some(now.saturating_sub(idle))
    }
}

#[cfg(not(windows))]
fn last_input(_now: Ms) -> Option<Ms> {
    None
}

/// Every 10 s: note the latest input; publish and save when an away spell starts or ends
/// (and save every few minutes, so a shutdown is counted as away from about the right time).
pub async fn watch(ctx: Ctx) {
    let mut saved_at: Ms = 0;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let now = now_ms();
        let Some(input) = last_input(now) else {
            continue;
        };
        let changed = ctx.presence.lock().unwrap().observe(input, now);
        if changed || now.saturating_sub(saved_at) > 5 * 60_000 {
            save(&ctx.presence.lock().unwrap());
            saved_at = now;
        }
        if changed {
            publish(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Ms = 60_000;

    #[test]
    fn away_spells_from_the_last_input() {
        let mut p = Presence::default();
        assert!(!p.observe(0, MIN));
        // Lunch: last input at 10 min, still nothing at 16 min.
        assert!(!p.observe(10 * MIN, 12 * MIN));
        assert!(p.observe(10 * MIN, 16 * MIN));
        assert_eq!(p.away, vec![(10 * MIN, None)]);
        assert!(!p.observe(10 * MIN, 50 * MIN), "still away");
        // Back at 70 min.
        assert!(p.observe(70 * MIN, 70 * MIN));
        assert_eq!(p.away, vec![(10 * MIN, Some(70 * MIN))]);
    }

    #[test]
    fn time_the_app_was_off_is_away() {
        let mut p = Presence {
            away: vec![],
            last_input: 100 * MIN,
        };
        p.resume(600 * MIN);
        assert_eq!(p.away, vec![(100 * MIN, None)]);
        // A quick restart isn't.
        let mut q = Presence {
            away: vec![],
            last_input: 100 * MIN,
        };
        q.resume(101 * MIN);
        assert!(q.away.is_empty());
    }

    #[test]
    fn old_spells_are_forgotten() {
        let mut p = Presence {
            away: vec![(0, Some(MIN))],
            last_input: MIN,
        };
        p.observe(KEEP + 2 * MIN, KEEP + 2 * MIN);
        assert!(p.away.is_empty());
    }
}
