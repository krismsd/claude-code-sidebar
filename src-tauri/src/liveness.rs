//! Liveness (SPEC §3.3): every 30 s, one `wsl.exe` call per distro checks all known claude
//! PIDs, never one call per session. Uses `/proc/<pid>` rather than `kill -0` (which fails with
//! EPERM for other users' processes), and `wsl.exe -e` (with `--` the default shell re-parses
//! the joined command line and the script's quoting is lost).
use crate::server::{Ctx, now_ms, publish};
use std::collections::HashMap;
use std::time::Duration;

const DEFAULT_DISTRO: &str = "Ubuntu";

pub async fn run(ctx: Ctx) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let pids = ctx.store.lock().unwrap().pids();
        if pids.is_empty() {
            continue;
        }
        let mut by_distro: HashMap<String, Vec<u32>> = HashMap::new();
        for (distro, pid) in pids {
            by_distro
                .entry(distro.unwrap_or_else(|| DEFAULT_DISTRO.into()))
                .or_default()
                .push(pid);
        }
        let mut dead = Vec::new();
        let mut changed = false;
        for (distro, pids) in by_distro {
            match check(&distro, &pids).await {
                Some(c) => {
                    dead.extend(c.dead);
                    changed |= ctx.store.lock().unwrap().set_versions(
                        &distro,
                        &c.versions,
                        c.latest.as_deref(),
                    );
                }
                None => log::warn!("liveness check failed for {distro}"),
            }
        }
        if !dead.is_empty() {
            ctx.store.lock().unwrap().pids_dead(&dead, now_ms());
            publish(&ctx);
            crate::server::sync_titles(&ctx, Duration::from_secs(1));
        } else if changed {
            publish(&ctx);
        }
    }
}

/// Every few seconds, for sessions hosted in tmux: is a terminal showing them? (Closing a tab
/// only detaches, so the card says when a session runs with nothing showing it.)
pub async fn watch_attached(ctx: Ctx) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(4)) => {}
            // A view closed: give its tmux client a moment to go, then look.
            _ = ctx.recheck.notified() => tokio::time::sleep(Duration::from_millis(300)).await,
        }
        let distros = ctx.store.lock().unwrap().tmux_distros();
        let mut changed = false;
        for distro in distros {
            let name = distro.clone().unwrap_or_else(|| DEFAULT_DISTRO.into());
            if let Some(list) = tmux_sessions(&name).await {
                changed |= ctx
                    .store
                    .lock()
                    .unwrap()
                    .set_attached(&distro, &list, now_ms());
            }
        }
        if changed {
            publish(&ctx);
        }
        // Nothing shows it and it has nothing to do: end it (SIGTERM, as End does). Its card
        // stays, closed and resumable; a click brings it back.
        let idle = ctx.store.lock().unwrap().take_idle_unwatched(now_ms());
        for (distro, pid) in idle {
            let name = distro.unwrap_or_else(|| DEFAULT_DISTRO.into());
            log::info!("ending idle background session (pid {pid})");
            let _ = wsl(&name, &format!("kill -TERM {pid}")).await;
        }
    }
}

/// `tmux ls` on the sidebar's socket: session name → has a client attached.
pub async fn tmux_sessions(distro: &str) -> Option<HashMap<String, bool>> {
    let out = wsl(
        distro,
        "tmux -L cc-sidebar ls -F '#{session_name} #{session_attached}' 2>/dev/null; true",
    )
    .await?;
    Some(
        out.lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(n, a)| (n.to_string(), a.trim() != "0"))
            .collect(),
    )
}

/// Run a small script in a distro, without a console window. None if it failed or hung.
async fn wsl(distro: &str, script: &str) -> Option<String> {
    wsl_for(distro, script, 10).await
}

/// `wsl` with a time limit in seconds.
pub async fn wsl_for(distro: &str, script: &str, secs: u64) -> Option<String> {
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(["-d", distro, "-e", "sh", "-c", script]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = tokio::time::timeout(Duration::from_secs(secs), cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What a liveness check found in one distro.
#[derive(Debug, Default, PartialEq)]
pub struct Check {
    /// PIDs that are gone.
    pub dead: Vec<u32>,
    /// The Claude Code version each live PID runs (the native install keeps each version in
    /// its own `versions/<x.y.z>` file).
    pub versions: HashMap<u32, String>,
    /// The version `claude` starts now (`~/.local/bin/claude` links to it).
    pub latest: Option<String>,
}

/// None if the check itself failed (then assume alive).
async fn check(distro: &str, pids: &[u32]) -> Option<Check> {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "for p in {list}; do if [ -d /proc/$p ]; then v=$(readlink /proc/$p/exe); \
         echo \"v $p ${{v##*/}}\"; else echo \"d $p\"; fi; done; \
         l=$(readlink -f \"$HOME/.local/bin/claude\"); echo \"l ${{l##*/}}\""
    );
    Some(parse_check(&wsl(distro, &script).await?))
}

/// The check's output: "d <pid>" (gone), "v <pid> <exe name>" (alive), "l <exe name>".
pub fn parse_check(out: &str) -> Check {
    // An exe whose file was replaced reads "2.1.326 (deleted)"; one that isn't a version
    // (an npm install runs node) says nothing.
    let version = |w: Option<&str>| w.filter(|v| parse_version(v).is_some()).map(String::from);
    let mut c = Check::default();
    for line in out.lines() {
        let mut w = line.split_whitespace();
        match (w.next(), w.next()) {
            (Some("d"), Some(p)) => c.dead.extend(p.parse::<u32>().ok()),
            (Some("v"), Some(p)) => {
                if let (Ok(p), Some(v)) = (p.parse(), version(w.next())) {
                    c.versions.insert(p, v);
                }
            }
            (Some("l"), v) => c.latest = version(v),
            _ => {}
        }
    }
    c
}

/// "2.1.326" → [2, 1, 326], for comparing.
pub fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<Vec<_>>>()
        .filter(|v| v.len() > 1)
}
