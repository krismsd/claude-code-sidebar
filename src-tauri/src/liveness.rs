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
        for (distro, pids) in by_distro {
            match check(&distro, &pids).await {
                Some(d) => dead.extend(d),
                None => log::warn!("liveness check failed for {distro}"),
            }
        }
        if !dead.is_empty() {
            ctx.store.lock().unwrap().pids_dead(&dead, now_ms());
            publish(&ctx);
            crate::server::sync_titles(&ctx, Duration::from_secs(1));
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
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(["-d", distro, "-e", "sh", "-c", script]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Returns the PIDs that are gone, or None if the check itself failed (then assume alive).
async fn check(distro: &str, pids: &[u32]) -> Option<Vec<u32>> {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("for p in {list}; do [ -d /proc/$p ] || echo $p; done");
    let out = wsl(distro, &script).await?;
    Some(
        out.split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect(),
    )
}
