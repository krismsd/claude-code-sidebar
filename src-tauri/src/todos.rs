//! Project to-do lists (SPEC §3.19): a running list of what to do next in a project, written
//! mostly by the user while sessions work, read and ticked off by sessions through an MCP
//! server (`/mcp`, reached from WSL via the `cc-mcp` relay).
use crate::session::Ms;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoState {
    Open,
    /// Copied into a prompt by the user: in flight, but nothing says where.
    Sent,
    /// A session said it's on it.
    Doing,
    Done,
}

impl TodoState {
    fn word(self) -> &'static str {
        match self {
            TodoState::Open => "open",
            TodoState::Sent => "sent",
            TodoState::Doing => "doing",
            TodoState::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    /// Unique within the project, never reused.
    pub id: u64,
    pub text: String,
    pub state: TodoState,
    /// The session that added it (its name when added); none: the user.
    #[serde(default)]
    pub added_by: Option<String>,
    /// The session working on it or that finished it.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    pub at: Ms,
    pub updated: Ms,
}

/// Who is calling the MCP server, as the `cc-mcp` relay reports it.
#[derive(Debug, Default, Clone)]
pub struct Caller {
    pub pid: Option<u32>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
}

/// The project and session a call is about.
#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub root: String,
    pub session_id: Option<String>,
    pub session_name: Option<String>,
}

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "The user's running to-do list for this project, kept in the \
cc-sidebar app: things they want done next, often noted while you were busy. Read it when \
they mention their to-dos, the list or what's next. Mark an item doing when you start it \
and done when it's finished. Add an item only for follow-up work the user should see (not \
your own steps).";

fn tools() -> Value {
    json!([
        {
            "name": "todo_list",
            "description": "The project's to-do list: open items in the user's order, with ids. \
                States: open, sent (the user pasted it into a prompt), doing (a session is on it), done.",
            "inputSchema": {"type": "object", "properties": {
                "include_done": {"type": "boolean", "description": "Also list finished items (the latest 20)."}
            }},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "todo_add",
            "description": "Add items to the end of the project's to-do list, for follow-up work \
                the user should see. One line each; links may be included.",
            "inputSchema": {"type": "object", "properties": {
                "items": {"type": "array", "items": {"type": "string"}, "minItems": 1}
            }, "required": ["items"]}
        },
        {
            "name": "todo_update",
            "description": "Mark an item doing (you've started it), done (finished) or open \
                (put back), and/or reword it.",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "integer"},
                "state": {"type": "string", "enum": ["open", "doing", "done"]},
                "text": {"type": "string"}
            }, "required": ["id"]}
        }
    ])
}

/// Where a call comes from: the session with the relay's session id, else the one whose
/// `claude` started the relay (its PID), else the tracked project holding the working directory.
pub fn resolve(store: &Store, caller: &Caller) -> Option<Resolved> {
    let session = caller
        .session_id
        .as_deref()
        .and_then(|id| store.get(id).cloned())
        .or_else(|| store.sessions_where(|s| caller.pid.is_some() && s.pid == caller.pid));
    if let Some(s) = session
        && let Some(root) = s.project.as_ref().map(|p| p.root.clone())
    {
        return Some(Resolved {
            root,
            session_name: Some(s.display_name()),
            session_id: Some(s.session_id),
        });
    }
    let cwd = caller.cwd.as_deref()?;
    let root = store
        .projects()
        .iter()
        .map(|t| t.project.root.as_str())
        .filter(|r| cwd == *r || cwd.starts_with(&format!("{r}/")))
        .max_by_key(|r| r.len())?;
    Some(Resolved {
        root: root.to_string(),
        session_id: None,
        session_name: None,
    })
}

/// One JSON-RPC message from the relay. None: a notification (no reply). The bool says
/// whether the list changed.
pub fn handle(store: &mut Store, caller: &Caller, msg: &Value, now: Ms) -> (Option<Value>, bool) {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let Some(id) = id else {
        return (None, false);
    };
    let reply = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    match method {
        "initialize" => {
            let version = msg["params"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOL);
            (
                Some(reply(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "cc-sidebar", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))),
                false,
            )
        }
        "ping" => (Some(reply(json!({}))), false),
        "tools/list" => (Some(reply(json!({"tools": tools()}))), false),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = &msg["params"]["arguments"];
            let (text, changed, error) = match resolve(store, caller) {
                None => (
                    format!(
                        "No cc-sidebar project for {}: is the sidebar tracking it?",
                        caller.cwd.as_deref().unwrap_or("this folder")
                    ),
                    false,
                    true,
                ),
                Some(at) => call(store, &at, name, args, now),
            };
            (
                Some(reply(json!({
                    "content": [{"type": "text", "text": text}],
                    "isError": error,
                }))),
                changed,
            )
        }
        _ => (
            Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {method}")}})),
            false,
        ),
    }
}

/// A tool call: (text for Claude, changed, is an error).
fn call(
    store: &mut Store,
    at: &Resolved,
    name: &str,
    args: &Value,
    now: Ms,
) -> (String, bool, bool) {
    match name {
        "todo_list" => {
            let all = args["include_done"].as_bool().unwrap_or(false);
            (list_text(store.todos(&at.root), all), false, false)
        }
        "todo_add" => {
            let items: Vec<String> = args["items"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).map(clean).collect())
                .unwrap_or_default();
            let items: Vec<String> = items.into_iter().filter(|t| !t.is_empty()).collect();
            if items.is_empty() {
                return ("Nothing to add: items is empty.".into(), false, true);
            }
            let ids: Vec<String> = items
                .into_iter()
                .filter_map(|t| store.todo_add(&at.root, t, at.session_name.clone(), now))
                .map(|id| format!("#{id}"))
                .collect();
            (format!("Added {}.", ids.join(", ")), !ids.is_empty(), false)
        }
        "todo_update" => {
            let Some(id) = args["id"].as_u64() else {
                return ("id is required.".into(), false, true);
            };
            let state = match args["state"].as_str() {
                None => None,
                Some("open") => Some(TodoState::Open),
                Some("doing") => Some(TodoState::Doing),
                Some("done") => Some(TodoState::Done),
                Some(s) => {
                    return (
                        format!("Unknown state {s}: use open, doing or done."),
                        false,
                        true,
                    );
                }
            };
            let text = args["text"].as_str().map(clean).filter(|t| !t.is_empty());
            let by = at.session_id.clone().zip(at.session_name.clone());
            match store.todo_update(&at.root, id, text, state, by, now) {
                Some(t) => (
                    format!("#{} is {}: {}", t.id, t.state.word(), t.text),
                    true,
                    false,
                ),
                None => (
                    format!("No item #{id} in this project's list."),
                    false,
                    true,
                ),
            }
        }
        _ => (format!("Unknown tool {name}."), false, true),
    }
}

/// One line, trimmed.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn list_text(todos: &[Todo], with_done: bool) -> String {
    let line = |t: &Todo| {
        let state = match (&t.state, &t.session_name) {
            (TodoState::Doing, Some(n)) => format!("doing: {n}"),
            (s, _) => s.word().to_string(),
        };
        format!("#{} [{state}] {}", t.id, t.text)
    };
    let open: Vec<String> = todos
        .iter()
        .filter(|t| t.state != TodoState::Done)
        .map(line)
        .collect();
    let mut out = if open.is_empty() {
        "The to-do list is empty.".to_string()
    } else {
        format!("{} to do:\n{}", open.len(), open.join("\n"))
    };
    if with_done {
        let mut done: Vec<&Todo> = todos
            .iter()
            .filter(|t| t.state == TodoState::Done)
            .collect();
        done.sort_by_key(|t| std::cmp::Reverse(t.updated));
        if !done.is_empty() {
            let lines: Vec<String> = done.into_iter().take(20).map(line).collect();
            out.push_str(&format!("\n\nDone:\n{}", lines.join("\n")));
        }
    }
    out
}

/// Items as a prompt to paste: one bullet each.
pub fn as_prompt(todos: &[&Todo]) -> String {
    todos
        .iter()
        .map(|t| format!("- {}", t.text))
        .collect::<Vec<_>>()
        .join("\n")
}
