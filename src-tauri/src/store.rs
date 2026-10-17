//! All known sessions, keyed by session_id, with ordering, retention and persistence rules
//! (SPEC §3.3, §3.9). Closed sessions stay until dismissed or pruned so they can be resumed.
use crate::events::{Event, Hook};
use crate::project::Project;
use crate::session::{Ms, Session, Status, Thresholds};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A project the user is tracking (SPEC §3.12): it stays in the sidebar with no sessions.
/// `project` is captured when tracked (no branch/worktree: those belong to sessions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tracked {
    pub project: Project,
    pub added_at: Ms,
}

pub struct Store {
    sessions: HashMap<String, Session>,
    /// Tracked projects in display order (the user reorders them; nothing reorders itself).
    projects: Vec<Tracked>,
    pub thresholds: Thresholds,
    /// Closed sessions are pruned after this long.
    pub closed_retention_ms: Ms,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            sessions: HashMap::new(),
            projects: Vec::new(),
            thresholds: Thresholds::default(),
            closed_retention_ms: 7 * 24 * 3_600_000,
        }
    }
}

impl Store {
    /// Apply a hook event. Returns false if it was ignored.
    pub fn apply(&mut self, hook: &Hook, now: Ms) -> bool {
        let id = &hook.common.session_id;
        // Our own calls, and unknown events for sessions we haven't seen, don't make cards.
        if hook.meta.internal
            || matches!(hook.event, Event::Other) && !self.sessions.contains_key(id)
        {
            return false;
        }
        // `/clear` ends this session and starts a new one: drop the old card.
        if let Event::SessionEnd { reason } = &hook.event
            && reason.as_deref() == Some("clear")
        {
            return self.sessions.remove(id).is_some();
        }
        let s = self
            .sessions
            .entry(id.clone())
            .or_insert_with(|| Session::new(id, now));
        s.apply(hook, now, &self.thresholds);
        if matches!(hook.event, Event::SessionEnd { .. }) && s.is_unused() {
            self.sessions.remove(id);
        }
        true
    }

    /// A session's project resolved: attach it, and track the project if it's new (it goes to
    /// the bottom), or refresh its name/colour (projects.toml may have changed).
    pub fn set_project(&mut self, session_id: &str, project: Project) {
        let Some(s) = self.sessions.get_mut(session_id) else {
            return;
        };
        let now = s.last_event_at;
        s.project = Some(project.clone());
        self.track(project, now);
    }

    /// Track a project (new ones go to the bottom) or refresh its name/colour.
    pub fn track(&mut self, project: Project, now: Ms) {
        let base = Project {
            worktree: None,
            branch: None,
            ..project
        };
        match self
            .projects
            .iter_mut()
            .find(|t| t.project.root == base.root)
        {
            Some(t) => t.project = base,
            None => self.projects.push(Tracked {
                project: base,
                added_at: now,
            }),
        }
    }

    /// Sessions with no name yet but a transcript, to backfill from it.
    pub fn unnamed(&self) -> Vec<(String, String, Option<String>)> {
        self.sessions
            .values()
            .filter(|s| s.title.is_none() && s.first_prompt.is_none())
            .filter_map(|s| {
                Some((
                    s.session_id.clone(),
                    s.transcript_path.clone()?,
                    s.distro.clone(),
                ))
            })
            .collect()
    }

    /// Fill in names read from a transcript, without overwriting what's already known.
    pub fn set_names(
        &mut self,
        session_id: &str,
        title: Option<String>,
        first_prompt: Option<String>,
    ) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            if s.title.is_none() {
                s.title = title;
            }
            if s.first_prompt.is_none() {
                s.first_prompt = first_prompt;
            }
        }
    }

    pub fn toggle_star(&mut self, session_id: &str) -> bool {
        self.sessions.get_mut(session_id).is_some_and(|s| {
            s.starred = !s.starred;
            true
        })
    }

    /// Drag-to-reorder: move a project to position `to` (clamped).
    pub fn move_project(&mut self, root: &str, to: usize) -> bool {
        let Some(from) = self.projects.iter().position(|t| t.project.root == root) else {
            return false;
        };
        let t = self.projects.remove(from);
        self.projects.insert(to.min(self.projects.len()), t);
        true
    }

    /// Stop tracking a project and drop its closed sessions (starred too). Refused while it
    /// still has open sessions.
    pub fn close_project(&mut self, root: &str) -> Result<(), String> {
        let in_project = |s: &Session| s.project.as_ref().is_some_and(|p| p.root == root);
        if self
            .sessions
            .values()
            .any(|s| in_project(s) && s.status != Status::Closed)
        {
            return Err("It still has open sessions. End or close them first.".into());
        }
        self.sessions.retain(|_, s| !in_project(s));
        let before = self.projects.len();
        self.projects.retain(|t| t.project.root != root);
        if self.projects.len() == before {
            return Err("Not a tracked project.".into());
        }
        Ok(())
    }

    pub fn projects(&self) -> &[Tracked] {
        &self.projects
    }

    pub fn restore_projects(&mut self, saved: Vec<Tracked>) {
        for t in saved {
            if !self
                .projects
                .iter()
                .any(|p| p.project.root == t.project.root)
            {
                self.projects.push(t);
            }
        }
    }

    pub fn set_haiku_recap(
        &mut self,
        session_id: &str,
        seq: u64,
        recap: crate::recap::Recap,
    ) -> bool {
        self.sessions
            .get_mut(session_id)
            .is_some_and(|s| s.set_haiku_recap(seq, recap))
    }

    pub fn permission_holdable(&mut self, session_id: &str, answerable: bool) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.permission_holdable(answerable);
        }
    }

    pub fn permission_answered(&mut self, session_id: &str, now: Ms) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.permission_answered(now);
        }
    }

    pub fn permission_released(&mut self, session_id: &str, now: Ms) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.permission_released(now);
        }
    }

    pub fn dismiss_alert(&mut self, session_id: &str, alert_id: u64) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.dismiss_alert(alert_id);
        }
    }

    /// Remove a closed session's card. Live sessions can't be dismissed.
    pub fn dismiss_session(&mut self, session_id: &str) -> bool {
        if self
            .sessions
            .get(session_id)
            .is_some_and(|s| s.status == Status::Closed)
        {
            self.sessions.remove(session_id);
            return true;
        }
        false
    }

    pub fn tick(&mut self, now: Ms) {
        for s in self.sessions.values_mut() {
            s.tick(now, &self.thresholds);
        }
        let keep = self.closed_retention_ms;
        self.sessions.retain(|_, s| {
            s.starred || s.status != Status::Closed || now.saturating_sub(s.status_since) < keep
        });
    }

    /// (distro, pid) of every open session we can check.
    pub fn pids(&self) -> Vec<(Option<String>, u32)> {
        self.sessions
            .values()
            .filter(|s| s.status != Status::Closed)
            .filter_map(|s| s.pid.map(|p| (s.distro.clone(), p)))
            .collect()
    }

    /// Result of a liveness check: these PIDs are gone.
    pub fn pids_dead(&mut self, dead: &[u32], now: Ms) {
        for s in self.sessions.values_mut() {
            if s.status != Status::Closed && s.pid.is_some_and(|p| dead.contains(&p)) {
                s.mark_lost(now);
            }
        }
        self.sessions
            .retain(|_, s| !(s.status == Status::Closed && s.is_unused()));
    }

    pub fn get(&self, session_id: &str) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    /// Display order: needs attention, working, idle, closed; stable by start time.
    pub fn ordered(&self) -> Vec<&Session> {
        let mut v: Vec<&Session> = self.sessions.values().collect();
        v.sort_by(|a, b| {
            (a.attention_rank(), a.started_at, &a.session_id).cmp(&(
                b.attention_rank(),
                b.started_at,
                &b.session_id,
            ))
        });
        v
    }

    /// Everything, for saving to disk.
    pub fn snapshot(&self) -> Vec<Session> {
        self.ordered().into_iter().cloned().collect()
    }

    /// Load saved sessions (app start). Open sessions without a PID can't be checked, so they
    /// start closed and revive on their next event; the rest are confirmed by liveness.
    pub fn restore(&mut self, saved: Vec<Session>, now: Ms) {
        for s in saved {
            let mut s = s.restored();
            if s.status != Status::Closed && s.pid.is_none() {
                s.mark_lost(now);
            }
            self.sessions.insert(s.session_id.clone(), s);
        }
    }
}
