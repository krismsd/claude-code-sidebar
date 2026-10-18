//! Subscription usage limits (the numbers `/usage` shows), shown at the foot of the sidebar.
//! Every `claude -p` call reports them in a `rate_limit_event` (documented as an Agent SDK
//! message type): we get them for free with each Haiku recap (`cc-recap`'s `USAGE:` line), plus
//! one tiny call at startup (`cc-usage`). Field names are camelCase in the CLI's output and
//! snake_case in the SDK docs, so both are accepted.
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
    /// "allowed", "allowed_warning" or "rejected".
    pub status: String,
    pub using_overage: bool,
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
    let (mut five, mut seven) = (None, None);
    if let Some(ws) = &raw.unified_windows {
        five = ws.get("five_hour").and_then(window);
        seven = ws.get("seven_day").and_then(window);
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
            _ => {}
        }
    }
    if five.is_none() && seven.is_none() {
        return None;
    }
    Some(Usage {
        five_hour: five,
        seven_day: seven,
        status: raw.status.unwrap_or_else(|| "allowed".into()),
        using_overage: raw.is_using_overage,
        updated_at: now,
    })
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
