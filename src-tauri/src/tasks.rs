//! Tasks from issue trackers (SPEC §3.12): `cc-tasks` asks Claude to read them through a
//! claude.ai connector; picking one starts a session with the task as its first prompt.
use crate::config::Config;
use crate::project::{self, TasksConfig};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: String,
    pub url: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Deserialize)]
struct Listing {
    tasks: Vec<Task>,
}

const DEFAULT_PROMPT: &str = "Work on {id}: {title}\n\n{summary}\n\nTicket: {url}\n\nStart by reading the full ticket, then propose a plan before changing code.";

/// Run `cc-tasks <connector> <query> [tools…]` in WSL from the project folder (so CLI
/// trackers can use its git remote). Takes ~10 s: it's a Claude call.
pub async fn list(cfg: &Config, root: &str, tc: &TasksConfig) -> Result<Vec<Task>, String> {
    let mut cmd = tokio::process::Command::new("wsl.exe");
    cmd.args(["--cd", root]);
    let script = format!("{} \"$@\"", cfg.tasks_command);
    cmd.args([
        "-d",
        &cfg.distro,
        "-e",
        "bash",
        "-lc",
        &script,
        "cc-tasks",
        &tc.connector,
        &tc.query,
    ]);
    cmd.args(&tc.tools);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(Duration::from_secs(150), cmd.output())
        .await
        .map_err(|_| "Timed out asking Claude for tasks.".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or("cc-tasks failed").to_string());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse(json: &str) -> Result<Vec<Task>, String> {
    serde_json::from_str::<Listing>(json)
        .map(|l| l.tasks)
        .map_err(|e| format!("Couldn't read the task list: {e}"))
}

/// The first prompt for a session started from `task`.
pub fn prompt(task: &Task, tc: Option<&TasksConfig>) -> String {
    tc.and_then(|t| t.prompt.as_deref())
        .unwrap_or(DEFAULT_PROMPT)
        .replace("{id}", &task.id)
        .replace("{title}", &task.title)
        .replace("{url}", &task.url)
        .replace("{summary}", &task.summary)
        .replace("{status}", &task.status)
}

/// Write the prompt into WSL's /tmp (via \\wsl.localhost) and start a session with it.
#[cfg(windows)]
pub fn start(
    cfg: &Config,
    root: &str,
    name: &str,
    task: &Task,
    tc: Option<&TasksConfig>,
) -> Result<Option<String>, String> {
    let safe_id: String = task
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let linux = format!("/tmp/cc-sidebar-task-{safe_id}-{nonce}.md");
    std::fs::write(project::windows_path(&cfg.distro, &linux), prompt(task, tc))
        .map_err(|e| e.to_string())?;
    crate::focus::launch(
        cfg,
        &cfg.distro,
        root,
        name,
        "",
        &[("CC_SIDEBAR_PROMPT_FILE", &linux)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cc_tasks_output() {
        let t = parse(r#"{"tasks":[{"id":"ACME-12","title":"Fix login","status":"In Progress","url":"https://x/browse/ACME-12","summary":"Users get logged out"},{"id":"ACME-13","title":"Docs","url":"https://x/browse/ACME-13"}]}"#).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].status, "");
        assert!(parse("not json").is_err());
    }

    #[test]
    fn prompt_template() {
        let t = Task {
            id: "ACME-12".into(),
            title: "Fix login".into(),
            status: "To Do".into(),
            url: "u".into(),
            summary: "s".into(),
        };
        assert!(prompt(&t, None).starts_with("Work on ACME-12: Fix login\n\ns\n\nTicket: u"));
        let tc = TasksConfig {
            connector: "A".into(),
            query: "q".into(),
            tools: vec![],
            prompt: Some("{id} [{status}]".into()),
        };
        assert_eq!(prompt(&t, Some(&tc)), "ACME-12 [To Do]");
    }
}
