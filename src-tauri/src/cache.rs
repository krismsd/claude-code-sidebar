//! Prompt cache (SPEC §3.15): Claude Code caches the conversation so far, and every request
//! refreshes it for its lifetime (1 hour, or 5 minutes; each request in the transcript says
//! which). Past that, the next prompt sends the whole context again at full price. Every 30 s,
//! for sessions not removed, read the end of the transcript (over \\wsl.localhost, only when
//! it has grown) for the last request's time, lifetime and size, and any compaction since.
use crate::server::{Ctx, publish};
use crate::session::Ms;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

const TAIL_BYTES: u64 = 2 << 20;
const HOUR: Ms = 3_600_000;
const FIVE_MIN: Ms = 300_000;

/// The cache as the last request left it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// When the last request was answered: the cache lives `ttl` from here.
    pub at: Ms,
    pub ttl: Ms,
    /// Tokens the next prompt sends (cached while warm).
    pub tokens: u64,
}

/// A compaction after the last request, while nothing else was going on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compacted {
    pub at: Ms,
    pub pre_tokens: u64,
    pub post_tokens: u64,
    /// Claude Code's idle compaction (before the cache expired), not a full context.
    pub idle: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Scan {
    pub cache: Option<Cache>,
    pub compacted: Option<Compacted>,
}

/// Read a transcript's tail (JSONL; the first line may be cut).
pub fn scan(text: &str) -> Scan {
    let mut out = Scan::default();
    let mut ttl = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let at = v["timestamp"].as_str().and_then(parse_time);
        match v["type"].as_str() {
            Some("assistant") => {
                let u = &v["message"]["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                if u.is_null()
                    || n("cache_read_input_tokens") + n("cache_creation_input_tokens") == 0
                {
                    continue;
                }
                // A request that only read the cache keeps the lifetime it was written with.
                let c = &u["cache_creation"];
                if c["ephemeral_1h_input_tokens"].as_u64().unwrap_or(0) > 0 {
                    ttl = Some(HOUR);
                } else if c["ephemeral_5m_input_tokens"].as_u64().unwrap_or(0) > 0 {
                    ttl = Some(FIVE_MIN);
                }
                let Some(at) = at else { continue };
                out.cache = Some(Cache {
                    at,
                    ttl: ttl.unwrap_or(FIVE_MIN),
                    tokens: n("input_tokens")
                        + n("cache_read_input_tokens")
                        + n("cache_creation_input_tokens"),
                });
                out.compacted = None;
            }
            Some("user") if is_prompt(&v) => out.compacted = None,
            Some("system") if v["subtype"] == "compact_boundary" => {
                let m = &v["compactMetadata"];
                out.compacted = at.map(|at| Compacted {
                    at,
                    pre_tokens: m["preTokens"].as_u64().unwrap_or(0),
                    post_tokens: m["postTokens"].as_u64().unwrap_or(0),
                    idle: false,
                });
            }
            Some("system")
                if v["subtype"] == "informational"
                    && v["content"].as_str().is_some_and(|t| {
                        t.starts_with("Compacted while idle")
                            || t.starts_with(
                                "Compacted using a summary written while you were away",
                            )
                    }) =>
            {
                if let Some(c) = &mut out.compacted {
                    c.idle = true;
                }
            }
            _ => {}
        }
    }
    out
}

/// A prompt the user typed (not a tool result, an injected notice or a compaction summary).
fn is_prompt(v: &Value) -> bool {
    if v["isCompactSummary"].as_bool() == Some(true) || v["isMeta"].as_bool() == Some(true) {
        return false;
    }
    match &v["message"]["content"] {
        Value::String(t) => !crate::session::injected(t),
        Value::Array(parts) => parts.iter().any(|p| {
            p["type"] == "text"
                && p["text"]
                    .as_str()
                    .is_some_and(|t| !crate::session::injected(t))
        }),
        _ => false,
    }
}

/// "2026-10-26T01:38:21.733Z" → epoch ms (transcripts are always UTC).
fn parse_time(s: &str) -> Option<Ms> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let ms = match s.get(19..20) {
        Some(".") => num(20..23).unwrap_or(0),
        _ => 0,
    };
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((((days * 24 + h) * 60 + mi) * 60 + sec) as Ms * 1000 + ms as Ms)
}

fn read(path: &std::path::Path, len: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub async fn watch(ctx: Ctx) {
    // Transcript length when last read, by session: unchanged files aren't read again.
    let mut seen: HashMap<String, u64> = HashMap::new();
    loop {
        let todo = ctx.store.lock().unwrap().transcripts();
        let ctx2 = ctx.clone();
        seen = tauri::async_runtime::spawn_blocking(move || {
            let mut changed = false;
            let mut next = HashMap::new();
            for (id, distro, transcript) in todo {
                let path = crate::project::windows_path(&distro, &transcript);
                let Some(len) = std::fs::metadata(&path).ok().map(|m| m.len()) else {
                    continue;
                };
                next.insert(id.clone(), len);
                if seen.get(&id) == Some(&len) {
                    continue;
                }
                if let Some(text) = read(&path, len) {
                    let mut store = ctx2.store.lock().unwrap();
                    changed |= store.set_cache(&id, scan(&text));
                    changed |= store.add_prs(&id, crate::prs::find(&text), false);
                }
            }
            if changed {
                publish(&ctx2);
            }
            next
        })
        .await
        .unwrap_or_default();
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(ts: &str, h1: u64, m5: u64, read: u64) -> String {
        serde_json::json!({"type": "assistant", "timestamp": ts, "isSidechain": false,
            "message": {"usage": {"input_tokens": 2, "cache_read_input_tokens": read,
                "cache_creation_input_tokens": h1 + m5,
                "cache_creation": {"ephemeral_1h_input_tokens": h1, "ephemeral_5m_input_tokens": m5}}}})
        .to_string()
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:00:01.500Z"), Some(1_500));
        assert_eq!(
            parse_time("2026-10-26T01:38:21.733Z"),
            Some(1_792_978_701_733)
        );
        assert_eq!(parse_time("nope"), None);
    }

    #[test]
    fn last_request_sets_expiry_and_size() {
        let t = [
            assistant("2026-10-26T01:00:00.000Z", 500, 0, 1000),
            // Only a read: keeps the 1 h lifetime.
            assistant("2026-10-26T01:10:00.000Z", 0, 0, 350_000),
        ]
        .join("\n");
        let c = scan(&t).cache.unwrap();
        assert_eq!(c.ttl, HOUR);
        assert_eq!(c.tokens, 350_002);
        assert_eq!(c.at, parse_time("2026-10-26T01:10:00.000Z").unwrap());
        let five = assistant("2026-10-26T01:00:00.000Z", 0, 900, 1000);
        assert_eq!(scan(&five).cache.unwrap().ttl, FIVE_MIN);
        // Subagents' requests are another cache.
        let side = five.replace("\"isSidechain\":false", "\"isSidechain\":true");
        assert_eq!(scan(&side).cache, None);
    }

    #[test]
    fn idle_compaction_after_the_last_request() {
        let t = [
            assistant("2026-10-26T01:38:21.733Z", 873, 0, 347_195),
            r#"{"type":"system","subtype":"compact_boundary","timestamp":"2026-10-26T02:33:06.960Z","compactMetadata":{"trigger":"auto","preTokens":350437,"postTokens":21000}}"#.into(),
            r#"{"type":"user","isCompactSummary":true,"timestamp":"2026-10-26T02:33:06.957Z","message":{"role":"user","content":"This session is being continued from a previous conversation"}}"#.into(),
            r#"{"type":"system","subtype":"informational","content":"Compacted while idle, before the prompt cache expired · turn off with \"idleCompaction\": false in settings.json","timestamp":"2026-10-26T02:33:07.382Z"}"#.into(),
        ]
        .join("\n");
        let s = scan(&t);
        assert_eq!(
            s.compacted,
            Some(Compacted {
                at: parse_time("2026-10-26T02:33:06.960Z").unwrap(),
                pre_tokens: 350_437,
                post_tokens: 21_000,
                idle: true,
            })
        );
        // The next prompt and its answer: old news.
        let later = format!(
            "{t}\n{}\n{}",
            r#"{"type":"user","timestamp":"2026-10-26T03:00:00.000Z","message":{"role":"user","content":"carry on"}}"#,
            assistant("2026-10-26T03:00:05.000Z", 22_000, 0, 0)
        );
        assert_eq!(scan(&later).compacted, None);
    }
}
