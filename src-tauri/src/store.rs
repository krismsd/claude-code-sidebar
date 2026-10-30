//! All known sessions, keyed by session_id, with ordering, retention and persistence rules
//! (SPEC §3.3, §3.9). Closed sessions stay until dismissed or pruned so they can be resumed.
use crate::events::{Event, Hook};
use crate::project::Project;
use crate::session::{AlertKind, Ms, Session, Status, Thresholds};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A project the user is tracking (SPEC §3.12): it stays in the sidebar with no sessions.
/// `project` is captured when tracked (no branch/worktree: those belong to sessions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tracked {
    pub project: Project,
    pub added_at: Ms,
    /// Put away in the drawer at the bottom of the sidebar (a small tile instead of a section).
    #[serde(default)]
    pub shelved: bool,
    /// Session ids in the order the user dragged them into. Sessions not listed (new ones)
    /// follow in their usual order.
    #[serde(default)]
    pub session_order: Vec<String>,
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
        // Starting a session in a project brings it back out of the drawer.
        let starting = s.status == Status::Starting;
        s.project = Some(project.clone());
        let root = project.root.clone();
        self.track(project, now);
        if starting {
            self.set_shelved(&root, false);
        }
    }

    /// Put a project in the drawer, or take it back out. False if nothing changed.
    pub fn set_shelved(&mut self, root: &str, shelved: bool) -> bool {
        match self.projects.iter_mut().find(|t| t.project.root == root) {
            Some(t) if t.shelved != shelved => {
                t.shelved = shelved;
                true
            }
            _ => false,
        }
    }

    /// The user dragged a project's sessions into this order.
    pub fn set_session_order(&mut self, root: &str, ids: Vec<String>) -> bool {
        match self.projects.iter_mut().find(|t| t.project.root == root) {
            Some(t) if t.session_order != ids => {
                t.session_order = ids;
                true
            }
            _ => false,
        }
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
                shelved: false,
                session_order: Vec::new(),
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

    /// Sessions whose card has no last answer yet (saved before it existed, or first seen
    /// mid-turn): (id, transcript, distro).
    pub fn unanswered(&self) -> Vec<(String, String, Option<String>)> {
        self.sessions
            .values()
            .filter(|s| s.last_answer.is_none() && !s.is_removed())
            .filter_map(|s| {
                Some((
                    s.session_id.clone(),
                    s.transcript_path.clone()?,
                    s.distro.clone(),
                ))
            })
            .collect()
    }

    /// The last answer read from a transcript, unless a Stop brought a newer one meanwhile.
    pub fn set_answer(&mut self, session_id: &str, msg: &str) {
        if let Some(s) = self.sessions.get_mut(session_id)
            && s.last_answer.is_none()
        {
            s.last_answer = Some(crate::session::answer_text(msg)).filter(|a| !a.is_empty());
            s.question = crate::session::ending_question(msg);
        }
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

    /// Set (or clear, with "") the sidebar name. Returns the session if it's closed and the
    /// name needs writing to its transcript now.
    pub fn rename(&mut self, session_id: &str, name: &str) -> Option<Session> {
        let s = self.sessions.get_mut(session_id)?;
        let name = name.trim();
        s.user_title = (!name.is_empty()).then(|| crate::events::truncate(name, 80));
        s.user_title_synced = false;
        (s.status == Status::Closed && s.user_title.is_some()).then(|| s.clone())
    }

    /// Closed sessions whose sidebar name still has to be written to the transcript.
    pub fn unsynced_titles(&self) -> Vec<Session> {
        self.sessions
            .values()
            .filter(|s| {
                s.status == Status::Closed && s.user_title.is_some() && !s.user_title_synced
            })
            .cloned()
            .collect()
    }

    pub fn mark_title_synced(&mut self, session_id: &str) {
        if let Some(s) = self.sessions.get_mut(session_id) {
            s.user_title_synced = true;
        }
    }

    /// Minimise (or expand) session cards. Returns whether any changed.
    pub fn set_minimized(&mut self, session_ids: &[String], minimized: bool) -> bool {
        let mut changed = false;
        for id in session_ids {
            if let Some(s) = self.sessions.get_mut(id)
                && s.minimized != minimized
            {
                s.minimized = minimized;
                changed = true;
            }
        }
        changed
    }

    /// Move a session to its project's "closed" section (its process is ended separately).
    pub fn set_removed(&mut self, session_id: &str, removed: bool) -> bool {
        self.sessions.get_mut(session_id).is_some_and(|s| {
            let changed = s.removed != Some(removed);
            s.removed = Some(removed);
            changed
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

    /// The user opened the session: "job finished" has been seen (its card shows no line to
    /// dismiss it from, so otherwise it would flash the project until the next turn).
    pub fn seen(&mut self, session_id: &str) -> bool {
        let Some(s) = self.sessions.get_mut(session_id) else {
            return false;
        };
        let before = s.alerts.len();
        s.alerts
            .retain(|a| a.kind != crate::session::AlertKind::Finished);
        s.alerts.len() != before
    }

    /// Delete a removed session's card for good. Live sessions can't be deleted.
    pub fn dismiss_session(&mut self, session_id: &str) -> bool {
        if self
            .sessions
            .get(session_id)
            .is_some_and(|s| s.status == Status::Closed && s.is_removed())
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
        // Only removed sessions go; the rest stay until the user removes them.
        self.sessions.retain(|_, s| {
            !s.is_removed()
                || s.status != Status::Closed
                || now.saturating_sub(s.status_since) < keep
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
    /// Output files of running background tasks in open sessions (see `jobs::watch`).
    /// Sessions whose whole transcript hasn't been searched for PRs: (id, distro, transcript).
    pub fn prs_unscanned(&self) -> Vec<(String, String, String)> {
        self.sessions
            .values()
            .filter(|s| !s.is_removed() && !s.prs_scanned)
            .filter_map(|s| {
                Some((
                    s.session_id.clone(),
                    s.distro.clone().unwrap_or_else(|| "Ubuntu".into()),
                    s.transcript_path.clone()?,
                ))
            })
            .collect()
    }

    /// PRs found in a session's transcript (`full`: all of it). Returns whether any were new.
    pub fn add_prs(&mut self, session_id: &str, found: Vec<crate::prs::Found>, full: bool) -> bool {
        let Some(s) = self.sessions.get_mut(session_id) else {
            return false;
        };
        if full {
            s.prs_scanned = true;
        }
        let mut changed = false;
        for f in found {
            match s.prs.iter_mut().find(|p| p.same(&f)) {
                // Saved before the folder was recorded.
                Some(p) if p.dir.is_none() && f.dir.is_some() => {
                    p.dir = f.dir;
                    changed = true;
                }
                Some(_) => {}
                None => {
                    s.prs.push(crate::prs::Pr::new(f));
                    changed = true;
                }
            }
        }
        // An Azure DevOps PR created without --org (az took it from the git remote): the
        // session's other PRs are in the same organisation.
        let known = s
            .prs
            .iter()
            .filter(|p| p.host == crate::prs::Host::Azure)
            .find_map(|p| p.org.clone());
        for p in s.prs.iter_mut() {
            if p.host == crate::prs::Host::Azure && p.org.is_none() && known.is_some() {
                p.org = known.clone();
                changed = true;
            }
        }
        // Found once without its organisation and again with it: keep the first.
        let mut kept: Vec<crate::prs::Pr> = Vec::new();
        for p in std::mem::take(&mut s.prs) {
            if kept.iter().any(|k| k.is(&p)) {
                changed = true;
            } else {
                kept.push(p);
            }
        }
        s.prs = kept;
        changed
    }

    /// Open PRs to look up: never looked at, not for `prs::EVERY`, or their session has
    /// finished a turn since. (session id, distro, session directory, PR).
    pub fn prs_due(&self, now: Ms) -> Vec<(String, String, Option<String>, crate::prs::Pr)> {
        let mut out = Vec::new();
        for s in self.sessions.values().filter(|s| !s.is_removed()) {
            for p in s.prs.iter().filter(|p| p.open()) {
                let due = p.checked_at == 0
                    || now.saturating_sub(p.checked_at) >= crate::prs::EVERY
                    || s.last_stop_at.is_some_and(|t| t > p.checked_at);
                if due {
                    let distro = s.distro.clone().unwrap_or_else(|| "Ubuntu".into());
                    out.push((s.session_id.clone(), distro, s.cwd.clone(), p.clone()));
                }
            }
        }
        out
    }

    /// A PR looked up (None: the lookup failed; try again later). Alerts when its checks
    /// start failing, changes are requested, or it's merged (not on the first look).
    pub fn set_pr_status(
        &mut self,
        session_id: &str,
        pr: &crate::prs::Pr,
        status: Option<crate::prs::Status>,
        now: Ms,
    ) {
        use crate::prs::{Checks, Review, State};
        let Some(s) = self.sessions.get_mut(session_id) else {
            return;
        };
        let Some(i) = s.prs.iter().position(|p| p.is(pr)) else {
            return;
        };
        let p = &mut s.prs[i];
        // Not looked up successfully before (a failed lookup still sets `checked_at`).
        let first = p.state.is_none();
        p.checked_at = now;
        let Some(st) = status else { return };
        let name = format!(
            "#{}{}",
            p.number,
            st.title
                .as_deref()
                .map(|t| format!(" {t}"))
                .unwrap_or_default()
        );
        let mut news = Vec::new();
        if !first {
            if st.checks == Some(Checks::Failing) && p.checks != Some(Checks::Failing) {
                news.push(format!("PR checks failed: {name}"));
            }
            if st.review == Some(Review::ChangesRequested)
                && p.review != Some(Review::ChangesRequested)
            {
                news.push(format!("Changes requested: {name}"));
            }
            if st.state == Some(State::Merged) && p.state != Some(State::Merged) {
                news.push(format!("PR merged: {name}"));
            }
        }
        p.state = st.state;
        p.checks = st.checks;
        p.review = st.review;
        if st.title.is_some() {
            p.title = st.title;
        }
        if st.url.is_some() {
            p.url = st.url;
        }
        if p.repo.is_none() {
            p.repo = p.url.as_deref().and_then(crate::prs::repo_from_url);
        }
        for text in news {
            s.alert(AlertKind::Pr, crate::events::truncate(&text, 100), now);
        }
    }

    /// Sessions not removed, with a transcript: (session id, distro, transcript path).
    pub fn transcripts(&self) -> Vec<(String, String, String)> {
        self.sessions
            .values()
            .filter(|s| !s.is_removed())
            .filter_map(|s| {
                let t = s.transcript_path.clone()?;
                Some((
                    s.session_id.clone(),
                    s.distro.clone().unwrap_or_else(|| "Ubuntu".into()),
                    t,
                ))
            })
            .collect()
    }

    /// What `cache::scan` found in a session's transcript. Returns whether it changed.
    pub fn set_cache(&mut self, session_id: &str, scan: crate::cache::Scan) -> bool {
        let Some(s) = self.sessions.get_mut(session_id) else {
            return false;
        };
        // A tail without any request (a long tool output) says nothing new about the cache.
        let cache = scan.cache.or_else(|| s.cache.clone());
        let changed = s.cache != cache || s.compacted != scan.compacted;
        s.cache = cache;
        s.compacted = scan.compacted;
        changed
    }

    pub fn job_files(&self) -> Vec<crate::jobs::JobFile> {
        let mut out = Vec::new();
        for s in self.sessions.values() {
            let Some(dir) = &s.tasks_dir else { continue };
            if s.status == Status::Closed {
                continue;
            }
            for b in s.background.iter().filter(|b| !b.id.is_empty()) {
                out.push(crate::jobs::JobFile {
                    session_id: s.session_id.clone(),
                    id: b.id.clone(),
                    distro: s.distro.clone().unwrap_or_else(|| "Ubuntu".into()),
                    path: format!("{dir}/{}.output", b.id),
                });
            }
        }
        out
    }

    /// Open sessions with background items lacking a task id, or an output folder (headless
    /// `claude -p` sessions report no scratchpad): (session id, distro, transcript path).
    pub fn jobs_without_ids(&self) -> Vec<(String, String, String)> {
        self.sessions
            .values()
            .filter(|s| {
                s.status != Status::Closed
                    && s.background
                        .iter()
                        .any(|b| b.id.is_empty() || (!b.agent && s.tasks_dir.is_none()))
            })
            .filter_map(|s| {
                Some((
                    s.session_id.clone(),
                    s.distro.clone().unwrap_or_else(|| "Ubuntu".into()),
                    s.transcript_path.clone()?,
                ))
            })
            .collect()
    }

    /// Ids (and the output folder) found for a session's background items, matched by
    /// description, the latest start winning.
    pub fn set_job_ids(&mut self, session_id: &str, found: &[crate::jobs::Started]) -> bool {
        let Some(s) = self.sessions.get_mut(session_id) else {
            return false;
        };
        let mut changed = false;
        for b in s.background.iter_mut() {
            let Some(f) = found
                .iter()
                .rev()
                .find(|f| f.id == b.id || (b.id.is_empty() && f.description == b.description))
            else {
                continue;
            };
            if b.id.is_empty() {
                b.id = f.id.clone();
                changed = true;
            }
            if s.tasks_dir.is_none() {
                s.tasks_dir = f.output.rsplit_once('/').map(|(d, _)| d.to_string());
                changed = true;
            }
        }
        changed
    }

    /// What `jobs::watch` read from a task's output. Returns whether the card changed.
    pub fn set_job_output(
        &mut self,
        session_id: &str,
        id: &str,
        line: Option<String>,
        progress: Vec<crate::session::Progress>,
        output_at: Option<Ms>,
        now: Ms,
    ) -> bool {
        self.sessions
            .get_mut(session_id)
            .is_some_and(|s| s.job_output(id, line, progress, output_at, now))
    }

    /// Distros with open sessions hosted in tmux (to ask which have a terminal attached).
    pub fn tmux_distros(&self) -> Vec<Option<String>> {
        let mut out: Vec<Option<String>> = Vec::new();
        for s in self.sessions.values() {
            if s.tmux.is_some() && s.status != Status::Closed && !out.contains(&s.distro) {
                out.push(s.distro.clone());
            }
        }
        out
    }

    /// tmux's answer for one distro: session name → has a client attached. Returns whether any
    /// card changed.
    pub fn set_attached(
        &mut self,
        distro: &Option<String>,
        attached: &HashMap<String, bool>,
        now: Ms,
    ) -> bool {
        let mut changed = false;
        for s in self.sessions.values_mut() {
            let Some(name) = &s.tmux else { continue };
            if &s.distro != distro || s.status == Status::Closed {
                continue;
            }
            let seen = attached.get(name).copied();
            if seen.is_some() && s.attached != seen {
                s.attached = seen;
                changed = true;
            }
            match seen {
                Some(false) if s.unattached_since.is_none() => s.unattached_since = Some(now),
                Some(true) => {
                    s.unattached_since = None;
                    s.view_closed = false;
                }
                _ => {}
            }
        }
        changed
    }

    /// The terminal window stopped showing tmux session `name`.
    pub fn view_closed(&mut self, name: &str) {
        for s in self.sessions.values_mut() {
            if s.tmux.as_deref() == Some(name) && s.status != Status::Closed {
                s.view_closed = true;
            }
        }
    }

    /// Sessions to end for sitting idle with nothing showing them (see
    /// `Session::idle_unwatched`): (distro, pid). Marks them so their close reads "idle".
    pub fn take_idle_unwatched(&mut self, now: Ms) -> Vec<(Option<String>, u32)> {
        let mut out = Vec::new();
        for s in self.sessions.values_mut() {
            if !s.ending_idle && s.idle_unwatched(now) {
                s.ending_idle = true;
                out.push((s.distro.clone(), s.pid.unwrap_or(0)));
            }
        }
        out
    }

    pub fn restore(&mut self, saved: Vec<Session>, now: Ms) {
        for s in saved {
            let mut s = s.restored();
            // Saved before the start directory was tracked, the cwd may have followed a `cd`.
            if let Some(dir) = s
                .cwd
                .as_deref()
                .and_then(|c| crate::session::start_dir(c, s.transcript_path.as_deref()))
            {
                s.cwd = Some(dir);
            }
            if s.status != Status::Closed && s.pid.is_none() {
                s.mark_lost(now);
            }
            self.sessions.insert(s.session_id.clone(), s);
        }
    }
}
