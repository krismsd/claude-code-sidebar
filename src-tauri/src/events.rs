//! Hook payloads as sent by Claude Code (via the `cc-hook` shim), parsed leniently: unknown
//! events and missing fields must never break the feed.
use serde::Deserialize;
use serde_json::Value;

/// Metadata the shim adds as headers (from the session's environment).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookMeta {
    pub tag: Option<String>,
    pub pid: Option<u32>,
    pub wt_session: Option<String>,
    pub distro: Option<String>,
    /// Set on our own Haiku recap calls; such events are dropped.
    pub internal: bool,
    /// tmux session hosting this Claude Code ("cc-<tag>"), when it runs in tmux.
    pub tmux: Option<String>,
}

impl HookMeta {
    /// Build from request headers (`X-CC-Tag`, `X-CC-Pid`, `X-WT-Session`, `X-WSL-Distro`,
    /// `X-CC-Internal`); `get` looks a header up case-insensitively. Empty values count as absent.
    pub fn from_headers(get: impl Fn(&str) -> Option<String>) -> HookMeta {
        let val = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        HookMeta {
            tag: val("x-cc-tag"),
            pid: val("x-cc-pid").and_then(|p| p.parse().ok()),
            wt_session: val("x-wt-session"),
            distro: val("x-wsl-distro"),
            internal: val("x-cc-internal").is_some_and(|v| v == "1"),
            tmux: val("x-cc-tmux"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Common {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `/tmp/claude-<uid>/<project>/<session>/scratchpad`: background tasks write their output
    /// next to it, in `tasks/<id>.output`.
    #[serde(default)]
    pub scratchpad_dir: Option<String>,
    /// Set when the hook fires inside a subagent (its tool calls, permission requests): the
    /// same id as the agent's entry in `background_tasks`.
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackgroundTask {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// "subagent" for agents, "shell" for background commands.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
}

impl BackgroundTask {
    pub fn running(&self) -> bool {
        self.status.as_deref() == Some("running")
    }

    pub fn is_agent(&self) -> bool {
        self.kind.as_deref().is_some_and(|k| k.contains("agent"))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum Event {
    SessionStart {
        #[serde(default)]
        source: Option<String>,
    },
    SessionEnd {
        #[serde(default)]
        reason: Option<String>,
    },
    UserPromptSubmit {
        #[serde(default)]
        prompt: String,
    },
    PreToolUse {
        tool_name: String,
        #[serde(default)]
        tool_input: Value,
    },
    PostToolUse {
        tool_name: String,
    },
    PostToolUseFailure {
        tool_name: String,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        is_interrupt: bool,
    },
    PermissionRequest {
        tool_name: String,
        #[serde(default)]
        tool_input: Value,
    },
    PermissionDenied {
        tool_name: String,
        #[serde(default)]
        tool_input: Value,
        #[serde(default)]
        reason: Option<String>,
    },
    Notification {
        #[serde(default)]
        notification_type: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    Stop {
        #[serde(default)]
        last_assistant_message: Option<String>,
        #[serde(default)]
        background_tasks: Vec<BackgroundTask>,
    },
    StopFailure {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        error_details: Option<String>,
    },
    PreCompact {
        #[serde(default)]
        trigger: Option<String>,
    },
    CwdChanged {
        new_cwd: String,
    },
    SubagentStart {
        #[serde(default)]
        agent_type: String,
    },
    SubagentStop {
        #[serde(default)]
        agent_type: String,
        /// What is still running in the background (absent in older Claude Code).
        #[serde(default)]
        background_tasks: Option<Vec<BackgroundTask>>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone)]
pub struct Hook {
    pub common: Common,
    pub event: Event,
    pub meta: HookMeta,
}

impl Hook {
    pub fn parse(body: &[u8], meta: HookMeta) -> Result<Hook, serde_json::Error> {
        let v: Value = serde_json::from_slice(body)?;
        Ok(Hook {
            common: serde_json::from_value(v.clone())?,
            event: serde_json::from_value(v)?,
            meta,
        })
    }
}

/// Short human description of a tool call, e.g. "Editing auth.ts" or "Running: npm test".
/// A tool call as a short trail item ("✎ auth.ts", "▶ cargo test"), or None for tools that
/// aren't steps of the work (questions, plans, task lists, tool search).
pub fn trail_item(tool: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let file = |k: &str| s(k).rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let short = |t: &str, n: usize| truncate(t, n);
    let item = match tool {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => format!("✎ {}", file("file_path")),
        "Read" => format!("📄 {}", file("file_path")),
        "Bash" => format!("▶ {}", command_words(s("command"))),
        "Grep" | "Glob" => format!("🔍 {}", short(s("pattern"), 20)),
        "WebFetch" => {
            let url = s("url");
            let host = url
                .split("://")
                .nth(1)
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or("");
            format!("🌐 {host}")
        }
        "WebSearch" => format!("🌐 {}", short(s("query"), 20)),
        "Agent" | "Task" => format!("🤖 {}", short(s("description"), 24)),
        "AskUserQuestion" | "ExitPlanMode" | "TodoWrite" | "ToolSearch" | "TaskCreate"
        | "TaskUpdate" | "TaskList" | "TaskStop" => return None,
        other => format!("⚙ {}", other.rsplit("__").next().unwrap_or(other)),
    };
    Some(item)
}

/// The gist of a shell command: its first two words, after any `cd … &&` and env settings
/// ("cd app && npm run test -- --watch" → "npm run").
fn command_words(cmd: &str) -> String {
    let last = cmd
        .split("&&")
        .map(str::trim)
        .find(|p| !p.starts_with("cd ") && !p.is_empty())
        .unwrap_or(cmd);
    let words: Vec<&str> = last
        .split_whitespace()
        .skip_while(|w| w.contains('=') && !w.starts_with('-'))
        .take(2)
        // A path: just its file name ("bash /tmp/x/shot.sh" → "bash shot.sh").
        .map(|w| w.rsplit('/').find(|p| !p.is_empty()).unwrap_or(w))
        .collect();
    truncate(&words.join(" "), 24)
}

pub fn describe_tool(tool: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let file = |k: &str| s(k).rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let text = match tool {
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => format!("Editing {}", file("file_path")),
        "Read" => format!("Reading {}", file("file_path")),
        "Bash" => {
            let d = s("description");
            if d.is_empty() {
                format!("Running: {}", s("command"))
            } else {
                d.to_string()
            }
        }
        "Grep" | "Glob" => format!("Searching {}", s("pattern")),
        "WebFetch" => format!("Fetching {}", s("url")),
        "WebSearch" => format!("Searching the web: {}", s("query")),
        "Agent" | "Task" => format!("Subagent: {}", s("description")),
        "AskUserQuestion" => {
            let q = input
                .pointer("/questions/0/question")
                .and_then(Value::as_str)
                .unwrap_or("");
            if q.is_empty() {
                "Asking you a question".to_string()
            } else {
                format!("Asks: {q}")
            }
        }
        other => other.trim_start_matches("mcp__").replace("__", " · "),
    };
    truncate(text.trim(), 80)
}

pub fn truncate(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        s
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}
