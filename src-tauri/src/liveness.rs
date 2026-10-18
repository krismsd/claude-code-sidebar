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

/// Returns the PIDs that are gone, or None if the check itself failed (then assume alive).
async fn check(distro: &str, pids: &[u32]) -> Option<Vec<u32>> {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("for p in {list}; do [ -d /proc/$p ] || echo $p; done");
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(["-d", distro, "-e", "sh", "-c", &script]);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect(),
    )
}
