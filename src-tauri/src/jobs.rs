//! Background job progress without asking Claude (SPEC §3.14): Claude Code writes each
//! background task's output to `<tasks_dir>/<id>.output` in WSL. Every few seconds, for open
//! sessions, read the end of each running task's file (over \\wsl.localhost) and keep its
//! latest line and any link in it (or in its command) on the card. A click opens the whole
//! output, following live, in the terminal window (`tail -f`).
use crate::server::{Ctx, publish};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

/// How much of the end of an output file to read for its latest line and links.
const TAIL_BYTES: u64 = 16 * 1024;

pub async fn watch(ctx: Ctx) {
    let mut backfilled = std::collections::HashSet::new();
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        // Background items recorded without a task id (by an older build, or while the app was
        // down): find their ids in the transcript, once per session.
        let missing: Vec<_> = ctx
            .store
            .lock()
            .unwrap()
            .jobs_without_ids()
            .into_iter()
            .filter(|(id, ..)| backfilled.insert(id.clone()))
            .collect();
        for (session_id, distro, transcript) in missing {
            let found = tauri::async_runtime::spawn_blocking(move || {
                ids_from_transcript(&crate::project::windows_path(&distro, &transcript))
            })
            .await
            .unwrap_or_default();
            if !found.is_empty() && ctx.store.lock().unwrap().set_job_ids(&session_id, &found) {
                publish(&ctx);
            }
        }
        let files = ctx.store.lock().unwrap().job_files();
        if files.is_empty() {
            continue;
        }
        let ctx = ctx.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let mut changed = false;
            for f in files {
                let text = read_tail(&crate::project::windows_path(&f.distro, &f.path));
                let line = text.as_deref().and_then(last_line);
                let url = text
                    .as_deref()
                    .and_then(last_url)
                    .or_else(|| last_url(&f.hint));
                changed |=
                    ctx.store
                        .lock()
                        .unwrap()
                        .set_job_output(&f.session_id, &f.id, line, url);
            }
            if changed {
                publish(&ctx);
            }
        })
        .await;
    }
}

/// A running background task's output file, for `watch`.
pub struct JobFile {
    pub session_id: String,
    pub id: String,
    pub distro: String,
    /// WSL path of the output file.
    pub path: String,
    /// Its command and description, to find a link in when the output has none.
    pub hint: String,
}

/// A background command found in a transcript: its description, task id and output file.
#[derive(Debug, Clone, PartialEq)]
pub struct Started {
    pub description: String,
    pub id: String,
    pub output: String,
}

/// Background commands started in a transcript, oldest first: the tool call (with
/// `run_in_background` and a description) and its result, "Command running in background
/// with ID: <id>. Output is being written to: <path>. …".
pub fn ids_from_transcript(path: &std::path::Path) -> Vec<Started> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    started_in(&text)
}

pub fn started_in(jsonl: &str) -> Vec<Started> {
    use serde_json::Value;
    let mut described: std::collections::HashMap<String, String> = Default::default();
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(content) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for c in content {
            match c.get("type").and_then(Value::as_str) {
                Some("tool_use")
                    if c.pointer("/input/run_in_background") == Some(&Value::Bool(true)) =>
                {
                    let (Some(id), Some(d)) = (
                        c.get("id").and_then(Value::as_str),
                        c.pointer("/input/description").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    described.insert(id.to_string(), d.to_string());
                }
                Some("tool_result") => {
                    let Some(d) = c
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|t| described.get(t))
                    else {
                        continue;
                    };
                    let text = match c.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(" "),
                        _ => continue,
                    };
                    let after = |key: &str| {
                        text.split_once(key).map(|(_, r)| {
                            r.trim_start()
                                .split(|c: char| c.is_whitespace())
                                .next()
                                .unwrap_or("")
                                .trim_end_matches('.')
                                .to_string()
                        })
                    };
                    if let (Some(id), Some(output)) =
                        (after("with ID:"), after("being written to:"))
                    {
                        out.push(Started {
                            description: d.clone(),
                            id,
                            output,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn read_tail(path: &std::path::Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The last non-empty line, as a terminal would show it: escape sequences removed, and only
/// what follows the last carriage return (progress bars rewrite their line).
pub fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(|l| {
            strip_ansi(l.rsplit('\r').next().unwrap_or(l))
                .trim()
                .to_string()
        })
        .find(|l| !l.is_empty())
        .map(|l| crate::events::truncate(&l, 160))
}

/// The last http(s) link in `text`, without trailing punctuation.
pub fn last_url(text: &str) -> Option<String> {
    let text = strip_ansi(text);
    text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
        .filter_map(|w| {
            w.find("https://")
                .or_else(|| w.find("http://"))
                .map(|i| &w[i..])
        })
        .map(|u| u.trim_end_matches(['.', ',', ';', ':', ')', ']', '}']))
        .rfind(|u| u.len() > "https://".len())
        .map(str::to_string)
}

/// Remove ANSI escape sequences (CSI, OSC and two-byte escapes).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte in @..~
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_line_as_shown() {
        let out = "step 1 ok\n\x1b[32mstep 2: running\x1b[0m\n\n";
        assert_eq!(last_line(out).as_deref(), Some("step 2: running"));
        assert_eq!(last_line("10%\r50%\r90%\n").as_deref(), Some("90%"));
        assert_eq!(last_line("\n  \n"), None);
    }

    #[test]
    fn started_commands_from_transcript() {
        let jsonl = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"sleep 9","description":"Wait for the deploy","run_in_background":true}}]}}"#,
            "
",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Command running in background with ID: bab12. Output is being written to: /tmp/claude-1000/-p/s1/tasks/bab12.output. You will be notified when it completes."}]}}"#,
            "
   
",
        );
        assert_eq!(
            started_in(jsonl),
            vec![Started {
                description: "Wait for the deploy".into(),
                id: "bab12".into(),
                output: "/tmp/claude-1000/-p/s1/tasks/bab12.output".into(),
            }]
        );
    }

    #[test]
    fn links() {
        let out = "queued https://dev.azure.com/org/proj/_build/results?buildId=1.\nsee (https://x.io/run/2)\n";
        assert_eq!(last_url(out).as_deref(), Some("https://x.io/run/2"));
        assert_eq!(
            last_url("\x1b]8;;https://a.b/c\x1b\\link\x1b]8;;\x1b\\ done"),
            None,
            "OSC 8 targets are not shown text"
        );
        assert_eq!(last_url("nothing here"), None);
    }
}
