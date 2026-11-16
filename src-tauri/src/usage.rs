//! Subscription usage limits (the numbers `/usage` shows), shown at the foot of the sidebar.
//! Every `claude -p` call reports them in a `rate_limit_event` (documented as an Agent SDK
//! message type): we get them for free with each Haiku recap (`cc-recap`'s `USAGE:` line), plus
//! one tiny call at startup (`cc-usage`). Field names are camelCase in the CLI's output and
//! snake_case in the SDK docs, so both are accepted.
//!
//! The weekly Fable limit (`seven_day_overage_included`, which Claude Code labels "Fable limit")
//! only comes with calls to Fable, so `cc-usage` asks Fable; the Haiku recaps don't carry it,
//! and [`merge`] keeps the last one seen.
use crate::session::Ms;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    /// 0–100.
    pub percent: f64,
    /// Epoch ms.
    pub resets_at: Option<Ms>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Usage {
    /// The 5-hour session window.
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// The weekly Fable limit, from the last event that had it.
    pub fable: Option<Window>,
    /// When `fable` was reported (epoch ms).
    pub fable_updated_at: Option<Ms>,
    /// "allowed", "allowed_warning" or "rejected".
    pub status: String,
    pub using_overage: bool,
    /// The limit `status` is about ("five_hour", "seven_day", "seven_day_overage_included", ...).
    pub limit_type: Option<String>,
    /// When these numbers were reported (epoch ms).
    pub updated_at: Ms,
}

#[derive(Deserialize)]
struct RawWindow {
    utilization: Option<f64>,
    #[serde(alias = "resetsAt")]
    resets_at: Option<f64>,
}

#[derive(Deserialize)]
struct RawInfo {
    #[serde(default)]
    status: Option<String>,
    #[serde(alias = "resetsAt")]
    resets_at: Option<f64>,
    #[serde(alias = "rateLimitType")]
    rate_limit_type: Option<String>,
    utilization: Option<f64>,
    #[serde(alias = "isUsingOverage", default)]
    is_using_overage: bool,
    #[serde(alias = "unifiedWindows")]
    unified_windows: Option<std::collections::HashMap<String, RawWindow>>,
}

/// The window Claude Code calls "Fable limit".
const FABLE: &str = "seven_day_overage_included";

/// Seconds or milliseconds since the epoch → ms.
fn to_ms(t: f64) -> Ms {
    if t > 1e12 {
        t as Ms
    } else {
        (t * 1000.0) as Ms
    }
}

/// 0–1 (SDK) or 0–100 → percent.
fn to_percent(u: f64) -> f64 {
    if u <= 1.0 { u * 100.0 } else { u }
}

/// Parse one `rate_limit_info` object.
pub fn parse(json: &str, now: Ms) -> Option<Usage> {
    let raw: RawInfo = serde_json::from_str(json.trim()).ok()?;
    let window = |w: &RawWindow| {
        w.utilization.map(|u| Window {
            percent: to_percent(u),
            resets_at: w.resets_at.map(to_ms),
        })
    };
    let (mut five, mut seven, mut fable) = (None, None, None);
    if let Some(ws) = &raw.unified_windows {
        five = ws.get("five_hour").and_then(window);
        seven = ws.get("seven_day").and_then(window);
        fable = ws.get(FABLE).and_then(window);
    }
    // Older/SDK shape: one window per event.
    if let (Some(kind), Some(u)) = (raw.rate_limit_type.as_deref(), raw.utilization) {
        let w = Some(Window {
            percent: to_percent(u),
            resets_at: raw.resets_at.map(to_ms),
        });
        match kind {
            "five_hour" if five.is_none() => five = w,
            "seven_day" if seven.is_none() => seven = w,
            FABLE if fable.is_none() => fable = w,
            _ => {}
        }
    }
    if five.is_none() && seven.is_none() && fable.is_none() {
        return None;
    }
    Some(Usage {
        five_hour: five,
        seven_day: seven,
        fable_updated_at: fable.as_ref().map(|_| now),
        fable,
        status: raw.status.unwrap_or_else(|| "allowed".into()),
        using_overage: raw.is_using_overage,
        limit_type: raw.rate_limit_type,
        updated_at: now,
    })
}

/// New numbers over old: an event without the Fable window (a Haiku call) keeps the last one.
pub fn merge(old: Option<&Usage>, mut new: Usage) -> Usage {
    if new.fable.is_none()
        && let Some(old) = old
    {
        new.fable = old.fable.clone();
        new.fable_updated_at = old.fable_updated_at;
    }
    new
}

/// When the oldest of the numbers shown was reported: the Fable window can lag the others.
pub fn oldest_update(u: &Usage) -> Ms {
    u.fable_updated_at
        .map_or(u.updated_at, |f| f.min(u.updated_at))
}

/// Split cc-recap output into (recap text, usage line JSON).
pub fn split_recap_output(out: &str) -> (String, Option<String>) {
    let mut usage = None;
    let text: Vec<&str> = out
        .lines()
        .filter(|l| match l.strip_prefix("USAGE:") {
            Some(j) => {
                usage = Some(j.trim().to_string());
                false
            }
            None => true,
        })
        .collect();
    (text.join("\n"), usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = r#"{"status":"allowed","resetsAt":1792294800,"rateLimitType":"five_hour","overageStatus":"rejected","overageDisabledReason":"org_level_disabled","isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.07,"resetsAt":1792294800},"seven_day":{"utilization":0.14,"resetsAt":1792458000}}}"#;

    #[test]
    fn parses_live_cli_output() {
        let u = parse(LIVE, 5).unwrap();
        let five = u.five_hour.unwrap();
        assert!((five.percent - 7.0).abs() < 1e-9);
        assert_eq!(five.resets_at, Some(1_792_294_800_000));
        assert!((u.seven_day.unwrap().percent - 14.0).abs() < 1e-9);
        assert_eq!(
            (u.status.as_str(), u.using_overage, u.updated_at),
            ("allowed", false, 5)
        );
    }

    /// From a Fable call (2.1.325): the Fable window comes along with the other two.
    const LIVE_FABLE: &str = r#"{"status":"allowed_warning","resetsAt":1794877200,"rateLimitType":"seven_day_overage_included","utilization":0.95,"overageStatus":"rejected","overageDisabledReason":"org_level_disabled","isUsingOverage":false,"surpassedThreshold":0.75,"unifiedWindows":{"five_hour":{"utilization":0.09,"resetsAt":1794810000,"status":"allowed"},"seven_day":{"utilization":0.75,"resetsAt":1794877200,"status":"allowed"},"seven_day_overage_included":{"utilization":0.95,"resetsAt":1794877200,"status":"allowed_warning","surpassedThreshold":0.75}},"overageEnabled":false}"#;

    #[test]
    fn parses_the_fable_window() {
        let u = parse(LIVE_FABLE, 7).unwrap();
        let fable = u.fable.clone().unwrap();
        assert!((fable.percent - 95.0).abs() < 1e-9);
        assert_eq!(fable.resets_at, Some(1_794_877_200_000));
        assert_eq!(u.fable_updated_at, Some(7));
        assert!((u.seven_day.unwrap().percent - 75.0).abs() < 1e-9);
        assert_eq!(u.limit_type.as_deref(), Some("seven_day_overage_included"));
        // A Haiku call has no Fable window.
        let haiku = parse(LIVE, 9).unwrap();
        assert!(haiku.fable.is_none() && haiku.fable_updated_at.is_none());
    }

    #[test]
    fn merge_keeps_the_last_fable_window() {
        let fable = parse(LIVE_FABLE, 7).unwrap();
        let merged = merge(Some(&fable), parse(LIVE, 9).unwrap());
        assert_eq!(merged.fable, fable.fable);
        assert_eq!(merged.fable_updated_at, Some(7));
        assert!((merged.five_hour.as_ref().unwrap().percent - 7.0).abs() < 1e-9);
        assert_eq!((merged.updated_at, oldest_update(&merged)), (9, 7));
        // A newer Fable window replaces it.
        let newer = merge(Some(&merged), parse(LIVE_FABLE, 11).unwrap());
        assert_eq!(newer.fable_updated_at, Some(11));
        // No Fable window ever: just the event's own time.
        assert_eq!(oldest_update(&parse(LIVE, 3).unwrap()), 3);
    }

    #[test]
    fn parses_sdk_shape_and_rejects_junk() {
        let u = parse(r#"{"status":"allowed_warning","rate_limit_type":"seven_day","utilization":0.91,"resets_at":1792458000}"#, 0).unwrap();
        assert!(u.five_hour.is_none());
        assert!((u.seven_day.unwrap().percent - 91.0).abs() < 1e-9);
        assert_eq!(u.status, "allowed_warning");
        assert!(parse("{}", 0).is_none());
        assert!(parse("nope", 0).is_none());
    }

    #[test]
    fn splits_usage_from_recap() {
        let (text, usage) = split_recap_output("TITLE: x\nDid y\n---\ndetail\nUSAGE: {\"a\":1}\n");
        assert_eq!(text, "TITLE: x\nDid y\n---\ndetail");
        assert_eq!(usage.as_deref(), Some("{\"a\":1}"));
        assert_eq!(split_recap_output("only text").1, None);
    }
}
