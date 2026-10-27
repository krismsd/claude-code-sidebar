//! Session model and status state machine (SPEC §3.3). Pure: time is passed in as epoch ms,
//! so everything here is driven by fixtures in unit tests.
use crate::events::{Event, Hook, describe_tool, truncate};
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Ms = u64;

/// Alert thresholds; configurable later (SPEC §3.10).
#[derive(Debug, Clone)]
pub struct Thresholds {
    /// ⏳ needs_permission / needs_input unanswered for this long.
    pub waiting_ms: Ms,
    /// 🐌 working with no events for this long.
    pub slow_ms: Ms,
    /// 🔁 this many consecutive failures of the same tool.
    pub repeat_failures: u32,
    /// After a held PermissionRequest is answered in the terminal, how long to wait for a tool
    /// result before concluding the user said no (no event is sent in that case).
    pub permission_grace_ms: Ms,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            waiting_ms: 2 * 60_000,
            slow_ms: 10 * 60_000,
            repeat_failures: 3,
            permission_grace_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Working,
    NeedsPermission,
    NeedsInput,
    /// "background" was a status for a while (2026-10-20); old saved sessions load as idle.
    #[serde(alias = "background")]
    Idle,
    Error,
    /// Claude Code has exited (SessionEnd) or its process is gone (liveness). The card stays so
    /// the session can be resumed; see `closed_reason`.
    Closed,
}

/// `closed_reason` when the process vanished without a SessionEnd (crash, kill).
pub const LOST: &str = "lost";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// 🚫 auto-mode denial
    Denied,
    /// ⚠️ StopFailure
    StopFailure,
    /// 🔁 repeated failures of one tool
    RepeatedFailure,
    /// ⏳ waiting on the user too long
    Waiting,
    /// 🐌 working with no events for a long time
    Slow,
    /// ✅ a job Claude was waiting on reported back and the session is ready for you
    Finished,
    /// 📦 context compaction (informational)
    Compact,
    /// 🔀 a PR the session opened: checks failed, changes requested, or merged
    Pr,
}

/// Steps kept in a turn's trail (the card shows the last few that fit).
const TRAIL_KEEP: usize = 8;

/// One step of a turn: a short label and how many times in a row it ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrailItem {
    pub label: String,
    pub count: u32,
}

/// Claude's task list: how far along it is, and the item in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todos {
    pub done: u32,
    pub total: u32,
    pub current: Option<String>,
}

impl Todos {
    /// TodoWrite's input: `todos: [{content, status, activeForm}]`.
    fn from_input(input: &serde_json::Value) -> Option<Todos> {
        let items = input.get("todos")?.as_array()?;
        let status = |t: &serde_json::Value| t["status"].as_str().unwrap_or("").to_string();
        let done = items.iter().filter(|t| status(t) == "completed").count() as u32;
        let total = items.len() as u32;
        if total == 0 || done == total {
            return None;
        }
        let current = items
            .iter()
            .find(|t| status(t) == "in_progress")
            .and_then(|t| {
                t["activeForm"]
                    .as_str()
                    .or_else(|| t["content"].as_str())
                    .map(|s| truncate(s, 80))
            });
        Some(Todos {
            done,
            total,
            current,
        })
    }
}

/// What an injected prompt is about: a background task's `<summary>`, else a generic note.
fn injected_note(prompt: &str) -> String {
    let summary = prompt
        .split_once("<summary>")
        .and_then(|(_, rest)| rest.split_once("</summary>"))
        .map(|(s, _)| s.trim());
    match summary {
        Some(s) if !s.is_empty() => truncate(s, 120),
        _ if prompt.contains("<task-notification>") => "A background task reported back".into(),
        _ => "Claude Code continued on its own".into(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: u64,
    pub kind: AlertKind,
    pub text: String,
    pub at: Ms,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPermission {
    pub tool: String,
    pub summary: String,
    pub since: Ms,
    /// The hook request is being held, so the card can approve/deny it (SPEC §3.6).
    #[serde(default)]
    pub answerable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Background {
    /// Claude Code's task id: its output is `<tasks_dir>/<id>.output`.
    #[serde(default)]
    pub id: String,
    pub agent: bool,
    pub description: String,
    #[serde(default)]
    pub command: String,
    /// Latest line of its output, read by `jobs::watch` while it runs.
    #[serde(default)]
    pub last_line: Option<String>,
    /// A job Claude waits on and will act on when it finishes (a background agent, a watcher,
    /// a poll loop), as opposed to a service it leaves running such as a dev server.
    #[serde(default)]
    pub awaited: bool,
}

const WAIT_WORDS: [&str; 10] = [
    "wait", "until", "poll", "watch", "monitor", "follow", "track", "finish", "complete", "status",
];

/// A background command that reads as a service Claude leaves running (a dev server, an app's
/// backend, a log tail). Judged from its description, which Claude writes for people, plus only
/// unmistakable server commands from the command itself: a long script is full of words like
/// "api" (`api-version=`) that mean nothing here. "api"/"backend"/"frontend"/"localhost" only
/// count when the description doesn't speak of waiting ("Watch the frontend deploy" is a job).
pub fn looks_service(description: &str, command: &str) -> bool {
    let d = description.to_lowercase();
    let c = command.to_lowercase();
    const SERVER_COMMANDS: [&str; 8] = [
        "npm run dev",
        "npm start",
        "vite",
        "tail -f",
        "docker compose up",
        "storybook",
        "uvicorn",
        "flask run",
    ];
    const SERVICE_WORDS: [&str; 3] = ["server", "serve", "watch mode"];
    const APP_PART: [&str; 4] = ["api", "backend", "frontend", "localhost"];
    let waits = WAIT_WORDS.iter().any(|w| d.contains(w));
    let words: Vec<&str> = d.split(|ch: char| !ch.is_ascii_alphanumeric()).collect();
    SERVER_COMMANDS
        .iter()
        .any(|w| d.contains(w) || c.contains(w))
        || SERVICE_WORDS.iter().any(|w| d.contains(w))
        || (!waits && APP_PART.iter().any(|w| words.contains(w)))
}

/// First guess, before Haiku's label: does this background command look like Claude waiting
/// for something to finish (a deploy watcher, a poll loop) rather than a service?
pub fn looks_awaited(description: &str, command: &str) -> bool {
    let d = description.to_lowercase();
    !looks_service(description, command) && WAIT_WORDS.iter().any(|w| d.contains(w))
}

/// A prompt Claude Code injected (`<task-notification>`, `<agent-message>`, …) rather than one
/// the user typed: they start with a tag.
pub fn injected(prompt: &str) -> bool {
    prompt.trim_start().starts_with('<')
}

/// Is a background task a job Claude waits on? Agents always are. Otherwise Haiku's label for
/// this task decides; without one, services (by their wording) aren't, and for the rest
/// Haiku's verdict on the session, else the guess from the wording, decides.
/// A background item Claude Code keeps itself rather than a command: no command, and its
/// description says what it listens for ("live updates for artifact …", "(re-armed on
/// session resume)").
fn subscription(description: &str, command: &str) -> bool {
    let d = description.to_lowercase();
    command.trim().is_empty()
        && (d.starts_with("live updates for") || d.contains("re-armed on session resume"))
}

fn awaited(
    agent: bool,
    description: &str,
    command: &str,
    label: Option<bool>,
    haiku: Option<bool>,
) -> bool {
    if agent {
        return true;
    }
    // A standing subscription Claude Code keeps itself (live updates for a published artifact):
    // it never finishes, so nothing waits on it.
    if subscription(description, command) {
        return false;
    }
    // Haiku read the conversation about this very task: its label wins.
    if let Some(label) = label {
        return label;
    }
    !looks_service(description, command)
        && haiku.unwrap_or_else(|| looks_awaited(description, command))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub tag: Option<String>,
    pub pid: Option<u32>,
    pub wt_session: Option<String>,
    pub distro: Option<String>,
    /// tmux session it runs in ("cc-<tag>"): closing its terminal only detaches.
    #[serde(default)]
    pub tmux: Option<String>,
    /// For a tmux session: whether any terminal shows it (None until checked).
    #[serde(default)]
    pub attached: Option<bool>,
    /// Since when no terminal has shown it (not saved).
    #[serde(skip)]
    pub unattached_since: Option<Ms>,
    /// The sidebar ended it for sitting idle with nothing showing it: its SessionEnd records
    /// "idle" rather than "other" (which reads as "tab closed").
    #[serde(skip)]
    pub ending_idle: bool,
    /// Its view in the sidebar's terminal window was just closed (not saved).
    #[serde(skip)]
    pub view_closed: bool,
    /// The directory the session was started in, which decides its project. The payload `cwd`
    /// follows the session's `cd`s, so this is recovered from the transcript path (`start_dir`).
    pub cwd: Option<String>,
    /// Where Claude's shell is now, from CwdChanged.
    pub shell_cwd: Option<String>,
    pub project: Option<Project>,
    pub transcript_path: Option<String>,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub status: Status,
    pub status_since: Ms,
    pub started_at: Ms,
    pub last_event_at: Ms,
    /// What it's doing right now, e.g. "Editing auth.ts".
    pub current_tool: Option<String>,
    pub last_prompt: Option<String>,
    /// This turn wasn't started by the user: what started it (a job reporting back).
    #[serde(default)]
    pub turn_note: Option<String>,
    /// This turn's steps so far, oldest first ("✎ auth.ts", "▶ cargo test"), with repeats of
    /// the same step counted.
    #[serde(default)]
    pub trail: Vec<TrailItem>,
    /// Claude's task list (TodoWrite), if it keeps one.
    #[serde(default)]
    pub todos: Option<Todos>,
    /// The first real prompt (truncated): the session's name until a title exists.
    #[serde(default)]
    pub first_prompt: Option<String>,
    /// Session name: a custom/ai title from the transcript, else a short Haiku title.
    #[serde(default)]
    pub title: Option<String>,
    /// Name given in the sidebar (wins over everything). Written into the transcript as a
    /// `custom-title` when the session is closed, so Claude Code's /resume shows it too.
    #[serde(default)]
    pub user_title: Option<String>,
    /// `user_title` has been written to the transcript.
    #[serde(default)]
    pub user_title_synced: bool,
    /// SessionStart source of the current run (startup, resume, clear, compact).
    #[serde(default)]
    pub start_source: Option<String>,
    /// Instant recap: first sentence of the last assistant message.
    pub recap: Option<String>,
    /// Claude's last answer, as written (Markdown markers removed): what the card shows.
    #[serde(default)]
    pub last_answer: Option<String>,
    /// The question it ended on, if any ("Shall I commit it?"): pinned under the answer.
    #[serde(default)]
    pub question: Option<String>,
    /// Haiku recap of the latest turn (SPEC §3.5); replaces `recap` on the card when present.
    #[serde(default)]
    pub recap_haiku: Option<String>,
    /// 2-3 lines of detail from the same Haiku call, for hover.
    #[serde(default)]
    pub recap_detail: Option<String>,
    /// Counts Stop events; a Haiku recap only applies to the Stop it was made for.
    #[serde(default)]
    pub stop_seq: u64,
    pub pending_permission: Option<PendingPermission>,
    /// Subagents started by the user-visible agent and still running.
    pub subagents_running: u32,
    /// Background shell commands (not agents) still running, as of the last report.
    pub background_running: u32,
    /// What runs in the background (agents and commands), as of the last Stop/SubagentStop.
    #[serde(default)]
    pub background: Vec<Background>,
    /// Haiku's verdict on the last Stop (cc-recap `WAITING:`): Claude will act on the result of
    /// its background commands. Beats the guess from their descriptions.
    #[serde(default)]
    pub jobs_awaited: Option<bool>,
    /// Jobs Claude was waiting on as of the last Stop (descriptions). One that's gone at the
    /// next Stop finished; if Claude, not the user, started that turn, it reported back.
    #[serde(default)]
    pub pending_jobs: Vec<String>,
    /// The current turn was started by the user's own prompt.
    #[serde(default)]
    pub user_turn: bool,
    /// A turn is running: from UserPromptSubmit to Stop.
    #[serde(default)]
    pub in_turn: bool,
    /// When the last turn ended: "ready" counts from here, through closing and resuming.
    #[serde(default)]
    pub last_stop_at: Option<Ms>,
    /// Where Claude Code writes background task output (`…/<session>/tasks`, WSL path).
    #[serde(default)]
    pub tasks_dir: Option<String>,
    /// Haiku's label for each background command it was shown, by description: a job Claude
    /// waits on (true) or a service left running (false). Kept while the task runs on.
    #[serde(default)]
    pub job_labels: std::collections::BTreeMap<String, bool>,
    pub alerts: Vec<Alert>,
    /// Removed by the user (or `/exit`): moved to its project's "closed" section, pruned after
    /// the retention period. Every other session stays in the list, running or not. None only
    /// in data saved before this existed (see `restored`).
    #[serde(default)]
    pub removed: Option<bool>,
    /// Minimised by the user: its card shows only the title bar (unless it needs them).
    #[serde(default)]
    pub minimized: bool,
    /// The prompt cache as the last request left it (read from the transcript, `cache`).
    #[serde(default)]
    pub cache: Option<crate::cache::Cache>,
    /// A compaction since the last request, e.g. Claude Code's idle compaction.
    #[serde(default)]
    pub compacted: Option<crate::cache::Compacted>,
    /// Pull requests the session opened (SPEC §3.16).
    #[serde(default)]
    pub prs: Vec<crate::prs::Pr>,
    /// The whole transcript has been searched for PRs (later, only its new tail is).
    #[serde(default)]
    pub prs_scanned: bool,
    /// Old: starred sessions were kept. Read once to migrate to `removed`.
    #[serde(default, skip_serializing)]
    pub starred: bool,
    /// Why it closed: the SessionEnd reason (`prompt_input_exit`, `other`, …) or `lost`.
    pub closed_reason: Option<String>,
    #[serde(skip)]
    tool_failures: HashMap<String, u32>,
    /// Set when a held PermissionRequest was released by the terminal (hook killed).
    #[serde(skip)]
    permission_released_at: Option<Ms>,
    /// What a background agent's permission request interrupted (status, since), restored once
    /// the agent carries on.
    #[serde(skip)]
    before_agent_permission: Option<(Status, Ms)>,
    #[serde(skip)]
    next_alert_id: u64,
}

impl Session {
    pub fn new(session_id: &str, now: Ms) -> Session {
        Session {
            session_id: session_id.to_string(),
            tag: None,
            pid: None,
            wt_session: None,
            distro: None,
            tmux: None,
            attached: None,
            unattached_since: None,
            ending_idle: false,
            view_closed: false,
            cwd: None,
            shell_cwd: None,
            project: None,
            transcript_path: None,
            permission_mode: None,
            model: None,
            status: Status::Starting,
            status_since: now,
            started_at: now,
            last_event_at: now,
            current_tool: None,
            last_prompt: None,
            turn_note: None,
            trail: Vec::new(),
            todos: None,
            first_prompt: None,
            title: None,
            user_title: None,
            user_title_synced: false,
            start_source: None,
            recap: None,
            last_answer: None,
            question: None,
            recap_haiku: None,
            recap_detail: None,
            stop_seq: 0,
            pending_permission: None,
            subagents_running: 0,
            background_running: 0,
            background: Vec::new(),
            jobs_awaited: None,
            pending_jobs: Vec::new(),
            user_turn: false,
            in_turn: false,
            last_stop_at: None,
            tasks_dir: None,
            job_labels: Default::default(),
            alerts: Vec::new(),
            closed_reason: None,
            removed: Some(false),
            minimized: false,
            cache: None,
            compacted: None,
            prs: Vec::new(),
            prs_scanned: false,
            starred: false,
            tool_failures: HashMap::new(),
            permission_released_at: None,
            before_agent_permission: None,
            next_alert_id: 1,
        }
    }

    fn set_status(&mut self, status: Status, now: Ms) {
        if status == Status::Closed {
            // Nothing to wait on or be slow about any more.
            self.alerts
                .retain(|a| !matches!(a.kind, AlertKind::Waiting | AlertKind::Slow));
        }
        if self.status != status {
            self.status = status;
            self.status_since = now;
        }
    }

    pub fn alert(&mut self, kind: AlertKind, text: String, now: Ms) {
        // One live alert per kind: refresh it rather than stacking duplicates.
        self.alerts.retain(|a| a.kind != kind);
        let id = self.next_alert_id;
        self.next_alert_id += 1;
        self.alerts.push(Alert {
            id,
            kind,
            text,
            at: now,
        });
    }

    /// After loading from disk: skipped fields are defaulted, so make alert ids unique again.
    pub fn restored(mut self) -> Session {
        // Saved by a build that never cleared it; a compaction doesn't outlive an app restart.
        self.alerts.retain(|a| a.kind != AlertKind::Compact);
        if self.status == Status::Closed {
            self.alerts
                .retain(|a| !matches!(a.kind, AlertKind::Waiting | AlertKind::Slow));
        }
        self.next_alert_id = self.alerts.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        // Saved before `removed`: closed sessions that weren't starred were in the closed
        // section; keep them there.
        if self.removed.is_none() {
            self.removed = Some(
                self.status == Status::Closed
                    && !self.starred
                    && self.closed_reason.as_deref() != Some("idle"),
            );
        }
        // Saved by a build that took injected prompts (task notifications) for the user's.
        if self.last_prompt.as_deref().is_some_and(injected) {
            self.last_prompt = None;
        }
        // Saved by a build that let Haiku's verdict mark services (dev servers) as jobs.
        let haiku = self.jobs_awaited;
        for b in &mut self.background {
            let label = self.job_labels.get(&b.description).copied();
            b.awaited = awaited(b.agent, &b.description, &b.command, label, haiku);
        }
        self.pending_jobs = self.awaited_jobs();
        self
    }

    /// A Haiku recap arrived for Stop number `seq`; ignored if a newer Stop happened since.
    pub fn set_haiku_recap(&mut self, seq: u64, recap: crate::recap::Recap) -> bool {
        if seq != self.stop_seq {
            return false;
        }
        self.recap_haiku = Some(recap.line);
        self.recap_detail = recap.detail;
        // Titles stick: a name stored in the transcript (a /rename) applies, but Haiku's own
        // suggestion only names a session that has no title yet, so a card doesn't change its
        // name as the conversation moves on.
        if recap.name.is_some() {
            self.title = recap.name;
        } else if self.title.is_none() {
            self.title = recap.title;
        }
        // Haiku read the turn: will Claude act on its background commands when they finish?
        if recap.waiting.is_some() || !recap.labels.is_empty() {
            if recap.waiting.is_some() {
                self.jobs_awaited = recap.waiting;
            }
            // Labels only for tasks still running; earlier ones are done with.
            self.job_labels
                .retain(|d, _| self.background.iter().any(|b| &b.description == d));
            self.job_labels.extend(recap.labels);
            for b in self.background.iter_mut() {
                let label = self.job_labels.get(&b.description).copied();
                b.awaited = awaited(
                    b.agent,
                    &b.description,
                    &b.command,
                    label,
                    self.jobs_awaited,
                );
            }
            self.pending_jobs = self.awaited_jobs();
        }
        true
    }

    fn awaited_jobs(&self) -> Vec<String> {
        self.background
            .iter()
            .filter(|b| b.awaited)
            .map(|b| b.description.clone())
            .collect()
    }

    /// A brand-new (or /clear-ed) session that ended before anything was asked: nothing to
    /// resume, so its card goes. Resumed sessions have history and are kept.
    pub fn is_removed(&self) -> bool {
        self.removed == Some(true)
    }

    pub fn is_unused(&self) -> bool {
        self.first_prompt.is_none()
            && matches!(self.start_source.as_deref(), Some("startup" | "clear"))
    }

    pub fn dismiss_alert(&mut self, id: u64) {
        self.alerts.retain(|a| a.id != id);
    }

    pub fn has_alert(&self, kind: AlertKind) -> bool {
        self.alerts.iter().any(|a| a.kind == kind)
    }

    /// Apply one hook event.
    pub fn apply(&mut self, hook: &Hook, now: Ms, th: &Thresholds) {
        let c = &hook.common;
        let m = &hook.meta;
        // SessionStart means a new claude process (fresh start, resume, /clear, compact):
        // take its identity as-is, including "none". A session resumed with plain `claude`
        // after a `claudes` run must not keep the old process's tag and PID, or liveness
        // checks the dead PID and closes the card. Other events only fill in gaps (the
        // app may start mid-session).
        if matches!(hook.event, Event::SessionStart { .. }) {
            self.tag = m.tag.clone();
            self.pid = m.pid;
            self.wt_session = m.wt_session.clone();
            self.tmux = m.tmux.clone();
        } else {
            if m.tmux.is_some() {
                self.tmux = m.tmux.clone();
            }
            if m.tag.is_some() {
                self.tag = m.tag.clone();
            }
            if m.pid.is_some() {
                self.pid = m.pid;
            }
            if m.wt_session.is_some() {
                self.wt_session = m.wt_session.clone();
            }
        }
        if m.distro.is_some() {
            self.distro = m.distro.clone();
        }
        if c.transcript_path.is_some() {
            self.transcript_path = c.transcript_path.clone();
        }
        if c.permission_mode.is_some() {
            self.permission_mode = c.permission_mode.clone();
        }
        if c.model.is_some() {
            self.model = c.model.clone();
        }
        if let Some(dir) = c
            .scratchpad_dir
            .as_deref()
            .and_then(|d| d.strip_suffix("/scratchpad"))
        {
            self.tasks_dir = Some(format!("{dir}/tasks"));
        }
        if let Some(cwd) = &c.cwd {
            match start_dir(cwd, self.transcript_path.as_deref()) {
                Some(dir) => self.cwd = Some(dir),
                // No transcript to go by: a new process starts where it was launched.
                None if self.cwd.is_none() || matches!(hook.event, Event::SessionStart { .. }) => {
                    self.cwd = Some(cwd.clone())
                }
                None => {}
            }
        }
        self.last_event_at = now;
        // Any sign of life brings a removed session back into the list (it was resumed).
        if !matches!(hook.event, Event::SessionEnd { .. }) {
            self.removed = Some(false);
        }
        // Any sign of life brings a closed session back (e.g. it was resumed).
        if self.status == Status::Closed && !matches!(hook.event, Event::SessionEnd { .. }) {
            self.closed_reason = None;
            self.set_status(Status::Idle, now);
        }
        self.alerts.retain(|a| a.kind != AlertKind::Slow);
        if !matches!(hook.event, Event::Notification { .. }) {
            self.permission_released_at = None;
        }

        if matches!(&hook.event, Event::UserPromptSubmit { prompt } if !injected(prompt)) {
            self.user_turn = true;
        }

        // A background agent works on after the turn: its tool calls aren't the session's turn,
        // so they leave the status (and the turn view) alone. Only its permission requests
        // need the user.
        let background_agent = !matches!(
            hook.event,
            Event::SubagentStart { .. } | Event::SubagentStop { .. }
        ) && c.agent_id.as_deref().is_some_and(|id| {
            self.status == Status::Idle || self.background.iter().any(|b| b.agent && b.id == id)
        });
        if background_agent {
            self.background_agent_event(&hook.event, now);
            return;
        }

        match &hook.event {
            Event::SessionStart { source } => {
                if source.as_deref() != Some("compact") {
                    self.start_source = source.clone();
                    self.in_turn = false;
                }
                match source.as_deref() {
                    // Compaction is done; the session carries on with whatever it was doing.
                    Some("compact") => self.alerts.retain(|a| a.kind != AlertKind::Compact),
                    Some("clear") => {
                        self.last_prompt = None;
                        self.recap = None;
                        self.current_tool = None;
                        self.set_status(Status::Idle, now);
                    }
                    // Resumed: a conversation with history, ready for the next prompt.
                    // Resumed: ready again, counting from when its last turn ended (opening it
                    // isn't news).
                    Some("resume") => {
                        self.set_status(Status::Idle, now);
                        if let Some(t) = self.last_stop_at {
                            self.status_since = t;
                        }
                    }
                    // New: ready, not yet prompted.
                    _ => self.set_status(Status::Starting, now),
                }
            }
            Event::SessionEnd { reason } => {
                // `/exit` is the user saying they're done with it.
                if reason.as_deref() == Some("prompt_input_exit") {
                    self.removed = Some(true);
                }
                let idle_end = std::mem::take(&mut self.ending_idle);
                self.closed_reason = Some(if idle_end {
                    "idle".into()
                } else {
                    reason.clone().unwrap_or_else(|| "other".into())
                });
                let ready_since = (self.status == Status::Idle).then_some(self.status_since);
                self.current_tool = None;
                self.pending_permission = None;
                self.subagents_running = 0;
                self.in_turn = false;
                self.set_background(&[]);
                self.set_status(Status::Closed, now);
                // Ended while ready (for sitting idle, or its tab closed): still the same ready
                // conversation, same timer.
                if let Some(t) = ready_since {
                    self.status_since = t;
                }
            }
            Event::UserPromptSubmit { prompt } => {
                // A new turn.
                self.trail.clear();
                self.turn_note = injected(prompt).then(|| injected_note(prompt));
                // Prompts Claude Code injects (an agent handing back, a background task
                // reporting it finished) start with a tag; they aren't the user's.
                if !injected(prompt) {
                    self.last_prompt = Some(truncate(prompt, 200));
                    if self.first_prompt.is_none() {
                        self.first_prompt = Some(truncate(prompt, 60));
                    }
                }
                self.pending_permission = None;
                self.current_tool = None;
                self.in_turn = true;
                self.set_status(Status::Working, now);
            }
            Event::PreToolUse {
                tool_name,
                tool_input,
            } => {
                // These tools wait on the user, and no other hook says so.
                let (doing, status) = match tool_name.as_str() {
                    "AskUserQuestion" => (describe_tool(tool_name, tool_input), Status::NeedsInput),
                    "ExitPlanMode" => (
                        "Waiting for you to approve the plan".to_string(),
                        Status::NeedsInput,
                    ),
                    _ => (describe_tool(tool_name, tool_input), Status::Working),
                };
                self.current_tool = Some(doing);
                self.set_status(status, now);
                if tool_name == "TodoWrite" {
                    self.todos = Todos::from_input(tool_input);
                }
                if let Some(label) = crate::events::trail_item(tool_name, tool_input) {
                    match self.trail.last_mut() {
                        Some(last) if last.label == label => last.count += 1,
                        _ => self.trail.push(TrailItem { label, count: 1 }),
                    }
                    let extra = self.trail.len().saturating_sub(TRAIL_KEEP);
                    self.trail.drain(..extra);
                }
            }
            Event::PostToolUse { tool_name } => {
                self.tool_failures.remove(tool_name);
                self.pending_permission = None;
                self.set_status(Status::Working, now);
            }
            Event::PostToolUseFailure {
                tool_name,
                error,
                is_interrupt,
            } => {
                self.pending_permission = None;
                if *is_interrupt {
                    self.current_tool = None;
                    self.set_status(Status::NeedsInput, now);
                } else {
                    let n = self.tool_failures.entry(tool_name.clone()).or_insert(0);
                    *n += 1;
                    let n = *n;
                    if n >= th.repeat_failures {
                        let why = error.as_deref().map(|e| format!(": {}", truncate(e, 80)));
                        self.alert(
                            AlertKind::RepeatedFailure,
                            format!(
                                "{tool_name} failed {n} times in a row{}",
                                why.unwrap_or_default()
                            ),
                            now,
                        );
                    }
                    self.set_status(Status::Working, now);
                }
            }
            // A question for the user also goes through the permission flow, but it isn't
            // an allow/deny decision: show it as needs-input with the question instead.
            Event::PermissionRequest {
                tool_name,
                tool_input,
            } if tool_name == "AskUserQuestion" => {
                self.pending_permission = None;
                self.current_tool = Some(describe_tool(tool_name, tool_input));
                self.set_status(Status::NeedsInput, now);
            }
            Event::PermissionRequest {
                tool_name,
                tool_input,
            } => {
                self.pending_permission = Some(PendingPermission {
                    tool: tool_name.clone(),
                    summary: describe_tool(tool_name, tool_input),
                    since: now,
                    answerable: false,
                });
                self.set_status(Status::NeedsPermission, now);
            }
            Event::PermissionDenied {
                tool_name,
                tool_input,
                reason,
            } => {
                let what = describe_tool(tool_name, tool_input);
                let text = match reason {
                    Some(r) => format!("Blocked: {what} ({})", r.trim()),
                    None => format!("Blocked: {what}"),
                };
                self.alert(AlertKind::Denied, text, now);
                // Claude carries on after an auto-mode denial.
                self.set_status(Status::Working, now);
            }
            Event::Notification {
                notification_type, ..
            } => match notification_type.as_deref() {
                // Also sent ~6 s into an AskUserQuestion: that stays a question (needs input).
                Some("permission_prompt")
                    if self.permission_released_at.is_none()
                        && self.status != Status::NeedsInput =>
                {
                    self.set_status(Status::NeedsPermission, now)
                }
                // Not `idle_prompt`: Claude Code sends it a minute after any finished turn,
                // which stays "ready".
                Some("elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input") => {
                    self.set_status(Status::NeedsInput, now)
                }
                _ => {}
            },
            Event::Stop {
                last_assistant_message,
                background_tasks,
            } => {
                // The Haiku recap described the previous turn; a new one is on its way.
                self.stop_seq += 1;
                self.last_stop_at = Some(now);
                self.recap_haiku = None;
                self.recap_detail = None;
                if let Some(msg) = last_assistant_message {
                    self.recap = Some(recap_text(msg));
                    self.last_answer = Some(answer_text(msg)).filter(|a| !a.is_empty());
                    self.question = ending_question(msg);
                }
                // A new turn: Haiku's verdict on the last one no longer applies.
                self.jobs_awaited = None;
                self.set_background(background_tasks);
                // The turn is over, so the only agents still going are background ones.
                self.subagents_running = self.background.iter().filter(|b| b.agent).count() as u32;
                self.current_tool = None;
                self.pending_permission = None;
                self.in_turn = false;
                self.set_status(Status::Idle, now);
                // A job Claude was waiting on is gone, and the turn wasn't the user's: it
                // reported back, and Claude has dealt with it. Ready for you right away.
                let now_running: Vec<&str> = self
                    .background
                    .iter()
                    .map(|b| b.description.as_str())
                    .collect();
                let finished: Vec<String> = std::mem::take(&mut self.pending_jobs)
                    .into_iter()
                    .filter(|d| !now_running.contains(&d.as_str()))
                    .collect();
                if !std::mem::take(&mut self.user_turn) && !finished.is_empty() {
                    let what = finished
                        .iter()
                        .find(|d| !d.is_empty())
                        .map_or("Background job".to_string(), |d| truncate(d, 80));
                    self.alert(AlertKind::Finished, format!("Finished: {what}"), now);
                }
                self.pending_jobs = self.awaited_jobs();
            }
            Event::StopFailure {
                error,
                error_details,
            } => {
                let text = match (error, error_details) {
                    (Some(e), Some(d)) => format!("{e}: {}", truncate(d, 100)),
                    (Some(e), None) => e.clone(),
                    (None, Some(d)) => truncate(d, 100),
                    (None, None) => "Stopped with an API error".into(),
                };
                self.alert(AlertKind::StopFailure, text, now);
                self.in_turn = false;
                self.current_tool = None;
                self.set_status(Status::Error, now);
            }
            Event::PreCompact { trigger } => {
                let text = if trigger.as_deref() == Some("auto") {
                    "Context full: compacting"
                } else {
                    "Compacting context"
                };
                self.alert(AlertKind::Compact, text.into(), now);
            }
            Event::CwdChanged { new_cwd } => self.shell_cwd = Some(new_cwd.clone()),
            // Empty agent_type = Claude Code's internal helper agents (prompt suggestions etc.).
            Event::SubagentStart { agent_type } if !agent_type.is_empty() => {
                self.subagents_running += 1;
                self.set_status(Status::Working, now);
            }
            Event::SubagentStop {
                agent_type,
                background_tasks,
            } => {
                if !agent_type.is_empty() {
                    self.subagents_running = self.subagents_running.saturating_sub(1);
                }
                if let Some(tasks) = background_tasks {
                    self.set_background(tasks);
                }
            }
            _ => {}
        }

        // Ended for sitting idle (see `idle_unwatched`) right after its job reported back: it is
        // still ready for the user, so the alert stays.
        if self.status != Status::Idle && self.closed_reason.as_deref() != Some("idle") {
            self.alerts.retain(|a| a.kind != AlertKind::Finished);
        }
    }

    fn background_agent_event(&mut self, event: &Event, now: Ms) {
        // "Working" with no turn running was set by this agent before its events were told
        // apart (an older build): it's ready, since the turn ended.
        if self.status == Status::Working && !self.in_turn {
            self.current_tool = None;
            self.status = Status::Idle;
            self.status_since = self.last_stop_at.unwrap_or(now);
        }
        match event {
            Event::PermissionRequest {
                tool_name,
                tool_input,
            } => {
                if self.before_agent_permission.is_none() {
                    self.before_agent_permission = Some((self.status, self.status_since));
                }
                self.pending_permission = Some(PendingPermission {
                    tool: tool_name.clone(),
                    summary: describe_tool(tool_name, tool_input),
                    since: now,
                    answerable: false,
                });
                self.set_status(Status::NeedsPermission, now);
            }
            Event::PermissionDenied {
                tool_name,
                tool_input,
                reason,
            } => {
                let what = describe_tool(tool_name, tool_input);
                let text = match reason {
                    Some(r) => format!("Blocked: {what} ({})", r.trim()),
                    None => format!("Blocked: {what}"),
                };
                self.alert(AlertKind::Denied, text, now);
            }
            // The agent carries on: back to what its request interrupted.
            Event::PreToolUse { .. }
            | Event::PostToolUse { .. }
            | Event::PostToolUseFailure { .. } => {
                if let Some((status, since)) = self.before_agent_permission.take() {
                    self.pending_permission = None;
                    self.permission_released_at = None;
                    self.status = status;
                    self.status_since = since;
                }
            }
            _ => {}
        }
    }

    fn set_background(&mut self, tasks: &[crate::events::BackgroundTask]) {
        let before = std::mem::take(&mut self.background);
        self.background = tasks
            .iter()
            .filter(|t| t.running())
            .map(|t| {
                let id = t.id.clone().unwrap_or_default();
                let description = t.description.clone().unwrap_or_default();
                let command = t.command.clone().unwrap_or_default();
                // Keep what was already read from its output.
                let seen = before.iter().find(|b| !id.is_empty() && b.id == id);
                Background {
                    agent: t.is_agent(),
                    awaited: awaited(
                        t.is_agent(),
                        &description,
                        &command,
                        self.job_labels.get(&description).copied(),
                        self.jobs_awaited,
                    ),
                    last_line: seen.and_then(|b| b.last_line.clone()),
                    id,
                    description,
                    command,
                }
            })
            .collect();
        self.background_running = self.background.iter().filter(|b| !b.agent).count() as u32;
    }

    /// A tmux-hosted session with no terminal showing it and nothing to do: no turn running, not
    /// waiting on the user, nothing in the background (jobs, dev servers, agents). Keeping its
    /// process buys nothing, so the sidebar ends it (it can be resumed). `GRACE` covers the
    /// moment after a click, before the new view attaches. A session never prompted whose view
    /// was just closed has nothing to come back to: it goes at once (its card goes with it).
    pub fn idle_unwatched(&self, now: Ms) -> bool {
        const GRACE: Ms = 5_000;
        let waited = |since: Ms| {
            (self.view_closed && self.is_unused()) || now.saturating_sub(since) >= GRACE
        };
        self.tmux.is_some()
            && self.pid.is_some()
            && self.attached == Some(false)
            && self.unattached_since.is_some_and(waited)
            && waited(self.last_event_at)
            && matches!(self.status, Status::Idle | Status::Starting | Status::Error)
            && self.pending_permission.is_none()
            && self.background.is_empty()
            && self.subagents_running == 0
    }

    /// The held request is live: the card may answer it.
    pub fn permission_holdable(&mut self, answerable: bool) {
        if let Some(p) = &mut self.pending_permission {
            p.answerable = answerable;
        }
    }

    /// Answered from the card: Claude proceeds (allow) or gets our message (deny) and goes on.
    pub fn permission_answered(&mut self, now: Ms) {
        self.pending_permission = None;
        self.permission_released_at = None;
        self.set_status(Status::Working, now);
    }

    /// A held PermissionRequest connection was dropped by Claude Code, meaning the user
    /// answered in the terminal. A tool result follows if they said yes; nothing if no.
    pub fn permission_released(&mut self, now: Ms) {
        if self.status == Status::NeedsPermission {
            self.pending_permission = None;
            self.permission_released_at = Some(now);
        }
    }

    /// Time-based transitions and alerts. Call periodically.
    pub fn tick(&mut self, now: Ms, th: &Thresholds) {
        if self.status == Status::Closed {
            return;
        }
        if let Some(at) = self.permission_released_at
            && now.saturating_sub(at) >= th.permission_grace_ms
        {
            // Said no in the terminal: Claude is waiting for new instructions.
            self.permission_released_at = None;
            self.current_tool = None;
            self.set_status(Status::NeedsInput, now);
        }
        let waiting = matches!(self.status, Status::NeedsPermission | Status::NeedsInput);
        let waited = now.saturating_sub(self.status_since);
        if waiting && waited >= th.waiting_ms && !self.has_alert(AlertKind::Waiting) {
            let mins = waited / 60_000;
            self.alert(
                AlertKind::Waiting,
                format!("Waiting for you for {mins}+ min"),
                now,
            );
        }
        if !waiting {
            self.alerts.retain(|a| a.kind != AlertKind::Waiting);
        }
        let quiet = now.saturating_sub(self.last_event_at);
        if self.status == Status::Working && quiet >= th.slow_ms && !self.has_alert(AlertKind::Slow)
        {
            self.alert(AlertKind::Slow, "No activity for 10+ min".into(), now);
        }
    }

    /// The process is gone without a SessionEnd.
    pub fn mark_lost(&mut self, now: Ms) {
        self.current_tool = None;
        self.pending_permission = None;
        self.subagents_running = 0;
        self.set_background(&[]);
        self.closed_reason = Some(LOST.into());
        self.set_status(Status::Closed, now);
    }

    /// Sort bucket: 0 needs attention, 1 working, 2 idle, 3 closed.
    pub fn attention_rank(&self) -> u8 {
        let alerting = self.alerts.iter().any(|a| a.kind != AlertKind::Compact);
        match self.status {
            Status::Closed => 3,
            Status::NeedsPermission | Status::NeedsInput | Status::Error => 0,
            _ if alerting => 0,
            Status::Working | Status::Starting => 1,
            _ => 2,
        }
    }
}

/// Names found in a transcript (JSONL text): the latest title Claude Code recorded (a
/// `custom-title` the user set beats an `ai-title`), and the first real user prompt.
/// Claude Code keeps a session's transcript in `~/.claude/projects/<dir>/`, where `<dir>` is the
/// directory it was started in with every non-alphanumeric character turned into `-`. The hook
/// `cwd` follows the session's `cd`s, so the start directory is `cwd` or the ancestor of it whose
/// name matches. None if nothing matches (no transcript yet, or a name too long to compare).
pub fn start_dir(cwd: &str, transcript_path: Option<&str>) -> Option<String> {
    let dir = transcript_path?.rsplit('/').nth(1)?;
    let encode = |p: &str| -> String {
        p.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    };
    let mut candidate = cwd.trim_end_matches('/');
    while !candidate.is_empty() {
        if encode(candidate) == dir {
            return Some(candidate.to_string());
        }
        candidate = &candidate[..candidate.rfind('/')?];
    }
    None
}

/// The text of the last assistant message in a transcript (JSONL; unreadable lines skipped).
pub fn last_assistant_text(jsonl: &str) -> Option<String> {
    jsonl.lines().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let text: Vec<&str> = v
            .pointer("/message/content")?
            .as_array()?
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect();
        Some(text.join("\n")).filter(|t| !t.trim().is_empty())
    })
}

pub fn transcript_names(jsonl: &str) -> (Option<String>, Option<String>) {
    let (mut custom, mut ai, mut first) = (None, None, None);
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        match v.get("type").and_then(|t| t.as_str()) {
            Some("custom-title") => custom = s("customTitle").or_else(|| s("title")).or(custom),
            Some("ai-title") => ai = s("aiTitle").or(ai),
            Some("user")
                if first.is_none() && v.get("isMeta").and_then(|m| m.as_bool()) != Some(true) =>
            {
                let c = &v["message"]["content"];
                let text = c.as_str().map(str::to_string).or_else(|| {
                    c.as_array()?.iter().find_map(|p| {
                        (p["type"] == "text").then(|| p["text"].as_str().unwrap_or("").to_string())
                    })
                });
                // Skip slash-command wrappers, hand-backs and other injected messages.
                first = text
                    .filter(|t| !t.trim().is_empty() && !t.trim_start().starts_with('<'))
                    .map(|t| truncate(&t, 60));
            }
            _ => {}
        }
    }
    (
        custom
            .or(ai)
            .filter(|t| !t.is_empty())
            .map(|t| truncate(&t, 60)),
        first,
    )
}

/// The instant recap: the start of the last assistant message as plain text (Markdown code,
/// emphasis and heading markers dropped, lines joined), up to ~400 characters. The card shows
/// up to five lines of it.
/// An assistant message as plain text for the card: Markdown emphasis, code markers, headings
/// and quotes removed, bullets as "•", code fences and blank lines dropped.
pub fn answer_text(msg: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for l in msg.lines() {
        let t = l.trim_end();
        if t.trim_start().starts_with("```") {
            continue;
        }
        let indent = t.len() - t.trim_start().len();
        let mut line = t.trim_start().replace("**", "").replace('`', "");
        line = line.trim_start_matches(['#', '>']).trim_start().to_string();
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            line = format!("{}• {rest}", " ".repeat(indent.min(6)));
        }
        if line.trim().is_empty() {
            continue;
        }
        out.push(line);
    }
    // Cut by length, keeping the lines (`truncate` would join them).
    let text = out.join("\n");
    match text.char_indices().nth(2000) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text,
    }
}

/// The question a message ends on: the last sentence of its last line, if it asks something.
pub fn ending_question(msg: &str) -> Option<String> {
    let text = answer_text(msg);
    let last = text.lines().rev().find(|l| !l.trim().is_empty())?.trim();
    let last = last.trim_start_matches("• ");
    if !last.ends_with('?') {
        return None;
    }
    // From the end of the previous sentence, if the line has several.
    let body = &last[..last.len() - 1];
    let start = body
        .char_indices()
        .filter(|&(i, c)| {
            matches!(c, '.' | '!' | '?' | ':') && body[i + c.len_utf8()..].starts_with(' ')
        })
        .map(|(i, c)| i + c.len_utf8())
        .next_back()
        .unwrap_or(0);
    Some(truncate(last[start..].trim(), 200))
}

pub fn recap_text(msg: &str) -> String {
    let plain: Vec<String> = msg
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("```"))
        .map(|l| l.replace("**", "").replace('`', ""))
        .map(|l| l.trim_start_matches(['#', '>']).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    truncate(&plain.join(" "), 400)
}

/// First sentence (or line) of an assistant message.
pub fn first_sentence(msg: &str) -> String {
    // Plain text for the card: drop Markdown code and emphasis markers.
    let msg = msg.replace("**", "").replace('`', "");
    let line = msg
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = line.trim_start_matches(['#', '*', '-', ' ']);
    let mut end = line.len();
    for (i, ch) in line.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let next = line[i + ch.len_utf8()..].chars().next();
            if next.is_none_or(char::is_whitespace) {
                end = i + ch.len_utf8();
                break;
            }
        }
    }
    truncate(&line[..end], 140)
}
