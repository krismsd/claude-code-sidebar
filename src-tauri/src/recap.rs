//! Haiku recaps (SPEC §3.5): after a Stop, run `cc-recap <transcript>` in WSL and swap the
//! result in for the instant recap. Debounced per session, rate-limited, and at most
//! MAX_PARALLEL calls at once.
use crate::server::{Ctx, now_ms, publish};
use crate::session::Ms;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Wait this long after a Stop before recapping, so quick back-and-forth turns coalesce.
const DEBOUNCE: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(90);
const MAX_PARALLEL: usize = 2;

#[derive(Default)]
struct PerSession {
    /// When the last recap call started.
    last_run: Option<Ms>,
    /// A task is already waiting or running for this session.
    busy: bool,
}

#[derive(Clone)]
pub struct Recapper {
    state: Arc<Mutex<HashMap<String, PerSession>>>,
    slots: Arc<Semaphore>,
}

impl Default for Recapper {
    fn default() -> Self {
        Recapper {
            state: Arc::default(),
            slots: Arc::new(Semaphore::new(MAX_PARALLEL)),
        }
    }
}

/// How long to wait before the next run, given the last run and the minimum interval.
pub fn wait_for(now: Ms, last_run: Option<Ms>, min_interval_ms: Ms) -> Ms {
    last_run.map_or(0, |t| (t + min_interval_ms).saturating_sub(now))
}

/// A Haiku recap: one line, optional detail, optional session title.
#[derive(Debug, Clone, PartialEq)]
pub struct Recap {
    pub line: String,
    pub detail: Option<String>,
    pub title: Option<String>,
    /// `WAITING: yes|no`: the turn ended waiting on background work that will resume it.
    pub waiting: Option<bool>,
}

/// Parse cc-recap output: optional `TITLE: …` and `WAITING: yes|no` lines, the recap line,
/// `---`, then detail.
pub fn parse(out: &str) -> Option<Recap> {
    let out = out.replace('`', "").replace("**", "");
    let mut parts = out.splitn(2, "\n---");
    let head = parts.next()?;
    let clean = |l: &str| {
        l.trim()
            .trim_matches(|c| c == '"' || c == '*')
            .trim()
            .to_string()
    };
    let mut title = None;
    let mut line = None;
    let mut waiting = None;
    for l in head.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(w) = l.strip_prefix("WAITING:") {
            waiting = Some(w.trim().to_ascii_lowercase().starts_with('y'));
        } else if let Some(t) = l
            .strip_prefix("TITLE:")
            .or_else(|| l.strip_prefix("Title:"))
        {
            title = Some(clean(t))
                .filter(|t| !t.is_empty())
                .map(|t| crate::events::truncate(&t, 60));
        } else if line.is_none() {
            line = Some(clean(l));
        }
    }
    let line = line.filter(|l| !l.is_empty() && l != "---")?;
    let detail = parts
        .next()
        .map(|d| {
            d.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && *l != "-")
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|d| !d.is_empty());
    Some(Recap {
        line: crate::events::truncate(&line, 110),
        detail,
        title,
        waiting,
    })
}

impl Recapper {
    /// Called on every Stop. Schedules one recap for the session unless one is already pending
    /// (that one will pick up the latest Stop when it runs).
    pub fn on_stop(&self, ctx: &Ctx, session_id: &str) {
        let cfg = ctx.config.read().unwrap().clone();
        if cfg.recap_command.is_empty() {
            return;
        }
        let last_run = {
            let mut st = self.state.lock().unwrap();
            let e = st.entry(session_id.to_string()).or_default();
            if e.busy {
                return;
            }
            e.busy = true;
            e.last_run
        };
        let (me, ctx, id) = (self.clone(), ctx.clone(), session_id.to_string());
        tauri::async_runtime::spawn(async move {
            let wait = wait_for(now_ms(), last_run, cfg.recap_min_interval_s * 1000);
            tokio::time::sleep(DEBOUNCE + Duration::from_millis(wait)).await;
            let _slot = me.slots.acquire().await;
            me.state
                .lock()
                .unwrap()
                .entry(id.clone())
                .or_default()
                .last_run = Some(now_ms());
            let rerun = me.run(&ctx, &id, &cfg).await;
            me.state.lock().unwrap().entry(id.clone()).or_default().busy = false;
            if rerun {
                me.on_stop(&ctx, &id);
            }
        });
    }

    /// Returns true if a newer turn finished while this ran, so it needs recapping too.
    async fn run(&self, ctx: &Ctx, id: &str, cfg: &crate::config::Config) -> bool {
        // Read the session as of now: the latest Stop is the one we recap.
        let Some((seq, transcript, distro)) = ctx.store.lock().unwrap().get(id).and_then(|s| {
            Some((
                s.stop_seq,
                s.transcript_path.clone()?,
                s.distro.clone().unwrap_or_else(|| cfg.distro.clone()),
            ))
        }) else {
            return false;
        };
        let mut cmd = tokio::process::Command::new("wsl.exe");
        // -e: no re-parsing by the default shell; bash -l for PATH (claude, cc-recap).
        cmd.args([
            "-d",
            &distro,
            "-e",
            "bash",
            "-lc",
            &format!("{} \"$1\"", cfg.recap_command),
            "cc-recap",
            &transcript,
        ]);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let started = std::time::Instant::now();
        let out = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
            Ok(Ok(o)) if o.status.success() => o,
            Ok(Ok(o)) => {
                let err = String::from_utf8_lossy(&o.stderr);
                log::warn!("recap {id}: exit {:?}: {}", o.status.code(), err.trim());
                return false;
            }
            Ok(Err(e)) => {
                log::warn!("recap {id}: {e}");
                return false;
            }
            Err(_) => {
                log::warn!("recap {id}: timed out");
                return false;
            }
        };
        let raw = String::from_utf8_lossy(&out.stdout);
        let (text, usage) = crate::usage::split_recap_output(&raw);
        if let Some(json) = usage {
            crate::server::set_usage(ctx, &json);
        }
        let Some(recap) = parse(&text) else {
            log::warn!("recap {id}: unparseable output: {}", text.trim());
            return false;
        };
        log::info!("recap {id} in {:.1}s", started.elapsed().as_secs_f32());
        let applied = ctx.store.lock().unwrap().set_haiku_recap(id, seq, recap);
        if applied {
            publish(ctx);
        }
        // Not applied means a newer turn finished while we ran.
        !applied
            && ctx
                .store
                .lock()
                .unwrap()
                .get(id)
                .is_some_and(|s| s.stop_seq > seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_detail_and_title() {
        let r = parse("TITLE: Shout helper for app.py\nDescribed the repo\n---\napp.py has two functions.\nREADME is a title.\n").unwrap();
        assert_eq!(r.title.as_deref(), Some("Shout helper for app.py"));
        assert_eq!(r.line, "Described the repo");
        assert_eq!(
            r.detail.as_deref(),
            Some("app.py has two functions.\nREADME is a title.")
        );
        let r = parse("\n\"Fixed the login bug\"\n").unwrap();
        assert_eq!(
            (r.line.as_str(), r.title, r.detail),
            ("Fixed the login bug", None, None)
        );
        assert!(parse("").is_none());
        assert!(parse("TITLE: only a title\n---\ndetail").is_none());
    }

    #[test]
    fn parses_waiting() {
        let r = parse("TITLE: Deploy\nWAITING: yes\nWatching the ML job\n---\nx\n").unwrap();
        assert_eq!(
            (r.line.as_str(), r.waiting),
            ("Watching the ML job", Some(true))
        );
        assert_eq!(parse("WAITING: no\nDone\n").unwrap().waiting, Some(false));
        assert_eq!(parse("Done\n").unwrap().waiting, None);
    }

    #[test]
    fn rate_limit() {
        assert_eq!(wait_for(100_000, None, 30_000), 0);
        assert_eq!(wait_for(100_000, Some(90_000), 30_000), 20_000);
        assert_eq!(wait_for(100_000, Some(50_000), 30_000), 0);
    }
}
