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
    /// 📦 context compaction (informational)
    Compact,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub tag: Option<String>,
    pub pid: Option<u32>,
    pub wt_session: Option<String>,
    pub distro: Option<String>,
    /// The session's project directory (the payload `cwd`; it does not follow `cd`).
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
    /// The first real prompt (truncated): the session's name until a title exists.
    #[serde(default)]
    pub first_prompt: Option<String>,
    /// Session name: a custom/ai title from the transcript, else a short Haiku title.
    #[serde(default)]
    pub title: Option<String>,
    /// SessionStart source of the current run (startup, resume, clear, compact).
    #[serde(default)]
    pub start_source: Option<String>,
    /// Instant recap: first sentence of the last assistant message.
    pub recap: Option<String>,
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
    /// Background tasks still running at the last Stop.
    pub background_running: u32,
    pub alerts: Vec<Alert>,
    /// Starred by the user: shown first in its project and never pruned (SPEC §3.12).
    #[serde(default)]
    pub starred: bool,
    /// Why it closed: the SessionEnd reason (`prompt_input_exit`, `other`, …) or `lost`.
    pub closed_reason: Option<String>,
    #[serde(skip)]
    tool_failures: HashMap<String, u32>,
    /// Set when a held PermissionRequest was released by the terminal (hook killed).
    #[serde(skip)]
    permission_released_at: Option<Ms>,
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
            first_prompt: None,
            title: None,
            start_source: None,
            recap: None,
            recap_haiku: None,
            recap_detail: None,
            stop_seq: 0,
            pending_permission: None,
            subagents_running: 0,
            background_running: 0,
            alerts: Vec::new(),
            closed_reason: None,
            starred: false,
            tool_failures: HashMap::new(),
            permission_released_at: None,
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

    fn alert(&mut self, kind: AlertKind, text: String, now: Ms) {
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
        if self.status == Status::Closed {
            self.alerts
                .retain(|a| !matches!(a.kind, AlertKind::Waiting | AlertKind::Slow));
        }
        self.next_alert_id = self.alerts.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        self
    }

    /// A Haiku recap arrived for Stop number `seq`; ignored if a newer Stop happened since.
    pub fn set_haiku_recap(&mut self, seq: u64, recap: crate::recap::Recap) -> bool {
        if seq != self.stop_seq {
            return false;
        }
        self.recap_haiku = Some(recap.line);
        self.recap_detail = recap.detail;
        if recap.title.is_some() {
            self.title = recap.title;
        }
        true
    }

    /// A brand-new (or /clear-ed) session that ended before anything was asked: nothing to
    /// resume, so its card goes. Resumed sessions have history and starred ones are kept.
    pub fn is_unused(&self) -> bool {
        !self.starred
            && self.first_prompt.is_none()
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
        } else {
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
        if c.cwd.is_some() {
            self.cwd = c.cwd.clone();
        }
        self.last_event_at = now;
        // Any sign of life brings a closed session back (e.g. it was resumed).
        if self.status == Status::Closed && !matches!(hook.event, Event::SessionEnd { .. }) {
            self.closed_reason = None;
            self.set_status(Status::Idle, now);
        }
        self.alerts.retain(|a| a.kind != AlertKind::Slow);
        if !matches!(hook.event, Event::Notification { .. }) {
            self.permission_released_at = None;
        }

        match &hook.event {
            Event::SessionStart { source } => {
                if source.as_deref() != Some("compact") {
                    self.start_source = source.clone();
                }
                match source.as_deref() {
                    // After compaction the session carries on with whatever it was doing.
                    Some("compact") => {}
                    Some("clear") => {
                        self.last_prompt = None;
                        self.recap = None;
                        self.current_tool = None;
                        self.set_status(Status::Idle, now);
                    }
                    // New or resumed: ready, not yet prompted.
                    _ => self.set_status(Status::Starting, now),
                }
            }
            Event::SessionEnd { reason } => {
                self.closed_reason = Some(reason.clone().unwrap_or_else(|| "other".into()));
                self.current_tool = None;
                self.pending_permission = None;
                self.subagents_running = 0;
                self.set_status(Status::Closed, now);
            }
            Event::UserPromptSubmit { prompt } => {
                // Background subagent hand-backs arrive as prompts; they aren't the user's.
                if !prompt.trim_start().starts_with("<agent-message") {
                    self.last_prompt = Some(truncate(prompt, 200));
                    if self.first_prompt.is_none() {
                        self.first_prompt = Some(truncate(prompt, 60));
                    }
                }
                self.pending_permission = None;
                self.current_tool = None;
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
                Some(
                    "idle_prompt"
                    | "elicitation_dialog"
                    | "elicitation_url_dialog"
                    | "agent_needs_input",
                ) => self.set_status(Status::NeedsInput, now),
                _ => {}
            },
            Event::Stop {
                last_assistant_message,
                background_tasks,
            } => {
                // The Haiku recap described the previous turn; a new one is on its way.
                self.stop_seq += 1;
                self.recap_haiku = None;
                self.recap_detail = None;
                if let Some(msg) = last_assistant_message {
                    self.recap = Some(recap_text(msg));
                }
                self.background_running = background_tasks
                    .iter()
                    .filter(|t| t.status.as_deref() == Some("running"))
                    .count() as u32;
                self.current_tool = None;
                self.pending_permission = None;
                self.set_status(Status::Idle, now);
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
            Event::SubagentStop { agent_type } if !agent_type.is_empty() => {
                self.subagents_running = self.subagents_running.saturating_sub(1);
            }
            _ => {}
        }
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
