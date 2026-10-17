//! Card and sidebar actions (SPEC §3.6, §3.11): interrupt/end a session, open its folder or
//! VS Code, list recent projects and start a new session.
use crate::config::Config;
use crate::project::{self, Overrides};
use crate::session::Session;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn distro<'a>(s: &'a Session, cfg: &'a Config) -> &'a str {
    s.distro.as_deref().unwrap_or(&cfg.distro)
}

/// Send a signal to the session's claude process: "INT" interrupts the current turn (like
/// Ctrl+C), "TERM" ends the session.
pub fn signal(s: &Session, cfg: &Config, sig: &str) -> Result<(), String> {
    let pid = s
        .pid
        .ok_or("This session wasn't started with claudes, so its process isn't known.")?;
    let status = hidden(Command::new("wsl.exe").args([
        "-d",
        distro(s, cfg),
        "-e",
        "kill",
        &format!("-{sig}"),
        &pid.to_string(),
    ]))
    .status()
    .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kill -{sig} {pid} failed"))
    }
}

pub fn open_folder(s: &Session, cfg: &Config) -> Result<(), String> {
    open_folder_at(distro(s, cfg), s.cwd.as_deref().ok_or("no folder")?)
}

pub fn open_vscode(s: &Session, cfg: &Config) -> Result<(), String> {
    open_vscode_at(distro(s, cfg), s.cwd.as_deref().ok_or("no folder")?)
}

/// Explorer on a WSL path (via \\wsl.localhost).
pub fn open_folder_at(distro: &str, path: &str) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(project::windows_path(distro, path))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// VS Code connected to WSL, opened on `path`.
pub fn open_vscode_at(distro: &str, path: &str) -> Result<(), String> {
    let remote = format!("wsl+{distro}");
    hidden(Command::new("cmd.exe").args(["/c", "code", "--remote", &remote, path]))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Recent {
    /// Linux path to start the session in (the project root).
    pub path: String,
    pub name: String,
    pub color: String,
    pub last_used: u64,
}

/// Map a folder picked in the Windows dialog back to a WSL path:
/// `\\wsl.localhost\<distro>\x\y` or `\\wsl$\<distro>\x\y` → `/x/y`; `C:\x` → `/mnt/c/x`.
pub fn linux_path(windows: &str) -> Option<String> {
    let w = windows.replace('/', "\\");
    for prefix in [r"\\wsl.localhost\", r"\\wsl$\"] {
        if let Some(rest) = w.strip_prefix(prefix) {
            let rest = rest.split_once('\\').map_or("", |(_, r)| r);
            return Some(
                format!("/{}", rest.replace('\\', "/"))
                    .trim_end_matches('/')
                    .to_string(),
            )
            .map(|p| if p.is_empty() { "/".into() } else { p });
        }
    }
    let mut chars = w.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(d), Some(':'), Some('\\') | None) if d.is_ascii_alphabetic() => {
            let rest = w[2..].trim_start_matches('\\').replace('\\', "/");
            Some(
                format!("/mnt/{}/{rest}", d.to_ascii_lowercase())
                    .trim_end_matches('/')
                    .to_string(),
            )
        }
        _ => None,
    }
}

/// The `cwd` recorded in a transcript (first lines that carry one).
fn transcript_cwd(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).ok()?;
    BufReader::new(f)
        .lines()
        .take(40)
        .map_while(Result::ok)
        .find_map(|l| {
            serde_json::from_str::<serde_json::Value>(&l)
                .ok()?
                .get("cwd")?
                .as_str()
                .map(String::from)
        })
}

/// Recent projects: from known sessions plus the newest transcript in each folder of
/// `~/.claude/projects` (found next to a known session's transcript_path). Newest first,
/// one entry per project root.
pub fn recent_projects(
    sessions: &[Session],
    cfg: &Config,
    overrides: &Overrides,
    limit: usize,
) -> Vec<Recent> {
    let mut seen: HashMap<String, u64> = HashMap::new();
    for s in sessions {
        if let Some(cwd) = &s.cwd {
            let e = seen.entry(cwd.clone()).or_default();
            *e = (*e).max(s.last_event_at);
        }
    }
    // ~/.claude/projects/<dir>/<id>.jsonl → ~/.claude/projects
    let projects_dir = sessions.iter().find_map(|s| {
        let t = s.transcript_path.as_deref()?;
        let dir = Path::new(t).parent()?.parent()?;
        Some(project::windows_path(
            distro(s, cfg),
            &dir.to_string_lossy(),
        ))
    });
    if let Some(dir) = projects_dir
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        for e in entries.flatten() {
            let newest = std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|f| f.path().extension().is_some_and(|x| x == "jsonl"))
                .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
                .max_by_key(|(t, _)| *t);
            if let Some((mtime, path)) = newest
                && let Some(cwd) = transcript_cwd(&path)
            {
                let ms = mtime
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let e = seen.entry(cwd).or_default();
                *e = (*e).max(ms);
            }
        }
    }
    let mut by_root: HashMap<String, Recent> = HashMap::new();
    for (cwd, last_used) in seen {
        let p = project::resolve(&cwd, |l| project::windows_path(&cfg.distro, l), overrides);
        if !project::windows_path(&cfg.distro, &p.root).is_dir() {
            continue; // deleted since
        }
        let r = by_root.entry(p.root.clone()).or_insert(Recent {
            path: p.root,
            name: p.name,
            color: p.color,
            last_used,
        });
        r.last_used = r.last_used.max(last_used);
    }
    let mut v: Vec<Recent> = by_root.into_values().collect();
    v.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.path.cmp(&b.path)));
    v.truncate(limit);
    v
}

/// Resolve a WSL folder the user picked into its project (repo root), checking it exists.
pub fn project_at(
    path: &str,
    cfg: &Config,
    overrides: &Overrides,
) -> Result<project::Project, String> {
    let path = path.trim().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') {
        return Err("Use a WSL path such as /home/you/dev/project".into());
    }
    if !project::windows_path(&cfg.distro, path).is_dir() {
        return Err(format!("{path} doesn't exist"));
    }
    Ok(project::resolve(
        path,
        |l| project::windows_path(&cfg.distro, l),
        overrides,
    ))
}

/// Start a new session in `path` (a WSL path that must exist).
#[cfg(windows)]
pub fn new_session(path: &str, cfg: &Config, overrides: &Overrides) -> Result<(), String> {
    let path = path.trim().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') {
        return Err("Use a WSL path such as /home/you/dev/project".into());
    }
    let dir: PathBuf = project::windows_path(&cfg.distro, path);
    if !dir.is_dir() {
        return Err(format!("{path} doesn't exist"));
    }
    let name = project::resolve(path, |l| project::windows_path(&cfg.distro, l), overrides).name;
    crate::focus::launch(cfg, &cfg.distro, path, &name, "", &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_to_linux_paths() {
        assert_eq!(
            linux_path(r"\\wsl.localhost\Ubuntu\home\u\dev\app").as_deref(),
            Some("/home/u/dev/app")
        );
        assert_eq!(
            linux_path(r"\\wsl$\Ubuntu\home\u\").as_deref(),
            Some("/home/u")
        );
        assert_eq!(linux_path(r"\\wsl.localhost\Ubuntu").as_deref(), Some("/"));
        assert_eq!(
            linux_path(r"C:\dev\cc-sidebar").as_deref(),
            Some("/mnt/c/dev/cc-sidebar")
        );
        assert_eq!(linux_path(r"D:\").as_deref(), Some("/mnt/d"));
        assert_eq!(linux_path("relative\\x"), None);
    }
}
