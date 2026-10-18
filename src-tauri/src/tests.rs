//! State machine tests driven by real hook payloads captured in dev/fixtures/ (M1 spikes).
use crate::events::{Event, Hook, HookMeta};
use crate::session::{AlertKind, Session, Status, Thresholds, first_sentence, recap_text};
use crate::store::Store;
use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../dev/fixtures");

fn raw(name: &str) -> Value {
    let path = format!("{FIXTURES}/{name}.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap()
}

fn hook_from(v: Value) -> Hook {
    let headers = v.get("_headers").cloned().unwrap_or(Value::Null);
    let meta = HookMeta::from_headers(|k| headers.get(k).and_then(Value::as_str).map(String::from));
    Hook::parse(v.to_string().as_bytes(), meta).unwrap()
}

fn fx(name: &str) -> Hook {
    hook_from(raw(name))
}

/// A fixture with fields overridden (e.g. to put events from different spikes in one session).
fn fx_with(name: &str, patch: Value) -> Hook {
    let mut v = raw(name);
    for (k, val) in patch.as_object().unwrap() {
        v[k] = val.clone();
    }
    hook_from(v)
}

const SID: &str = "test-session";

fn ours(name: &str) -> Hook {
    fx_with(name, serde_json::json!({ "session_id": SID }))
}

fn th() -> Thresholds {
    Thresholds::default()
}

#[test]
fn every_fixture_parses_into_a_known_event() {
    for entry in std::fs::read_dir(FIXTURES).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        let hook = fx(&name);
        assert!(
            !matches!(hook.event, Event::Other),
            "{name} parsed as Other"
        );
        let event_name = name.split('.').next().unwrap();
        assert!(
            format!("{:?}", hook.event).starts_with(event_name),
            "{name}: {:?}",
            hook.event
        );
    }
}

#[test]
fn headers_become_meta() {
    let h = fx("SessionStart");
    assert_eq!(h.meta.tag.as_deref(), Some("s1test"));
    assert_eq!(h.meta.pid, Some(382308));
    assert_eq!(h.meta.wt_session.as_deref(), Some("fake-wt-s1"));
    assert!(!h.meta.internal);
    assert!(fx("SessionEnd.internal-call").meta.internal);
}

#[test]
fn basic_lifecycle() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("SessionStart"), 1, &th());
    assert_eq!(s.status, Status::Starting);
    assert_eq!(s.pid, Some(382308));
    assert_eq!(
        s.cwd.as_deref(),
        Some("/home/user/cc-sidebar-sandbox/charlie")
    );

    s.apply(&ours("UserPromptSubmit"), 2, &th());
    assert_eq!(s.status, Status::Working);
    assert!(
        s.last_prompt
            .as_deref()
            .unwrap()
            .starts_with("Do these steps in order")
    );

    s.apply(&ours("PreToolUse"), 3, &th());
    assert_eq!(s.current_tool.as_deref(), Some("Reading app.py"));

    s.apply(&ours("PermissionRequest"), 4, &th());
    assert_eq!(s.status, Status::NeedsPermission);
    assert_eq!(s.pending_permission.as_ref().unwrap().tool, "Bash");

    s.apply(&ours("PostToolUse"), 5, &th());
    assert_eq!(s.status, Status::Working);
    assert!(s.pending_permission.is_none());

    s.apply(&ours("Stop"), 6, &th());
    assert_eq!(s.status, Status::Idle);
    assert!(s.current_tool.is_none());
    assert!(s.recap.is_some());
    assert_eq!(s.status_since, 6);

    s.apply(&ours("SessionEnd"), 7, &th());
    assert_eq!(s.status, Status::Closed);
    assert_eq!(s.closed_reason.as_deref(), Some("prompt_input_exit"));
    assert_eq!(s.attention_rank(), 3);

    // Resumed: any event revives it.
    s.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": SID, "source": "resume"}),
        ),
        8,
        &th(),
    );
    assert_eq!(s.status, Status::Starting);
    assert_eq!(s.closed_reason, None);
}

#[test]
fn agent_handback_is_not_the_users_prompt() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    let before = s.last_prompt.clone();
    s.apply(&ours("UserPromptSubmit.agent-handback"), 2, &th());
    assert_eq!(s.last_prompt, before);
    assert_eq!(s.status, Status::Working);
}

#[test]
fn internal_helper_agents_are_ignored() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("SubagentStart"), 1, &th());
    assert_eq!(s.subagents_running, 1);
    // Fixture SubagentStop.internal-agent has agent_type "" (prompt-suggestion helper).
    s.apply(&ours("SubagentStop.internal-agent"), 2, &th());
    assert_eq!(s.subagents_running, 1);
    s.apply(
        &fx_with(
            "SubagentStop",
            serde_json::json!({"session_id": SID, "agent_type": "general-purpose"}),
        ),
        3,
        &th(),
    );
    assert_eq!(s.subagents_running, 0);
}

#[test]
fn stop_with_background_tasks() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("Stop.background-tasks"), 1, &th());
    assert_eq!(s.status, Status::Idle);
    assert_eq!(s.background_running, 1);
    s.apply(&ours("Stop"), 2, &th());
    assert_eq!(s.background_running, 0);
}

#[test]
fn auto_mode_denial_raises_alert() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("PermissionDenied.auto-classifier"), 1, &th());
    let a = s
        .alerts
        .iter()
        .find(|a| a.kind == AlertKind::Denied)
        .expect("denied alert");
    assert!(a.text.contains("[Auto-Mode Bypass]"), "{}", a.text);
    assert!(a.text.contains("Print a test string"), "{}", a.text);
    assert_eq!(s.attention_rank(), 0);
    let id = a.id;
    s.dismiss_alert(id);
    assert!(s.alerts.is_empty());
}

#[test]
fn repeated_tool_failures() {
    let mut s = Session::new(SID, 0);
    for t in 1..=2 {
        s.apply(&ours("PostToolUseFailure"), t, &th());
    }
    assert!(!s.has_alert(AlertKind::RepeatedFailure));
    s.apply(&ours("PostToolUseFailure"), 3, &th());
    assert!(s.has_alert(AlertKind::RepeatedFailure));
    // A success resets the count (the alert itself stays until dismissed).
    s.apply(&ours("PostToolUse"), 4, &th()); // Read succeeded, Bash count untouched
    s.apply(
        &fx_with(
            "PostToolUse",
            serde_json::json!({"session_id": SID, "tool_name": "Bash"}),
        ),
        5,
        &th(),
    );
    s.alerts.clear();
    s.apply(&ours("PostToolUseFailure"), 6, &th());
    assert!(!s.has_alert(AlertKind::RepeatedFailure));
}

#[test]
fn permission_answered_no_in_terminal() {
    let t = th();
    let mut s = Session::new(SID, 0);
    s.apply(&ours("PermissionRequest"), 1_000, &t);
    s.apply(&ours("Notification"), 7_000, &t);
    assert_eq!(s.status, Status::NeedsPermission);
    s.permission_released(8_000);
    s.tick(9_000, &t);
    assert_eq!(s.status, Status::NeedsPermission, "still within grace");
    s.tick(8_000 + t.permission_grace_ms, &t);
    assert_eq!(s.status, Status::NeedsInput);
    assert!(s.pending_permission.is_none());
}

#[test]
fn permission_answered_yes_in_terminal() {
    let t = th();
    let mut s = Session::new(SID, 0);
    s.apply(&ours("PermissionRequest"), 1_000, &t);
    s.permission_released(2_000);
    s.apply(
        &fx_with(
            "PostToolUse",
            serde_json::json!({"session_id": SID, "tool_name": "Bash"}),
        ),
        2_100,
        &t,
    );
    s.tick(10_000, &t);
    assert_eq!(s.status, Status::Working);
}

#[test]
fn waiting_and_slow_alerts() {
    let t = th();
    let mut s = Session::new(SID, 0);
    s.apply(&ours("PermissionRequest"), 0, &t);
    s.tick(t.waiting_ms - 1, &t);
    assert!(!s.has_alert(AlertKind::Waiting));
    s.tick(t.waiting_ms, &t);
    assert!(s.has_alert(AlertKind::Waiting));
    let mut closed = s.clone();
    closed.apply(&ours("SessionEnd"), t.waiting_ms + 1, &t);
    assert!(!closed.has_alert(AlertKind::Waiting), "cleared on close");
    s.apply(&ours("PostToolUse"), t.waiting_ms + 1, &t);
    s.tick(t.waiting_ms + 2, &t);
    assert!(!s.has_alert(AlertKind::Waiting), "cleared once answered");

    let start = t.waiting_ms + 1;
    s.tick(start + t.slow_ms, &t);
    assert!(s.has_alert(AlertKind::Slow));
    s.apply(&ours("PreToolUse"), start + t.slow_ms + 1, &t);
    assert!(!s.has_alert(AlertKind::Slow), "cleared by any event");
}

#[test]
fn stop_failure_from_docs_shape() {
    // Not captured in M1 (needs a real API error); shape from the hooks reference.
    let h = hook_from(serde_json::json!({
        "session_id": SID, "hook_event_name": "StopFailure",
        "error": "rate_limit", "error_details": "429 Too Many Requests"
    }));
    let mut s = Session::new(SID, 0);
    s.apply(&h, 1, &th());
    assert_eq!(s.status, Status::Error);
    assert!(s.has_alert(AlertKind::StopFailure));
}

#[test]
fn compaction_keeps_status_and_flags() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    s.apply(&ours("PreCompact"), 2, &th());
    s.apply(&ours("SessionStart.compact"), 3, &th());
    assert_eq!(s.status, Status::Working);
    assert!(s.has_alert(AlertKind::Compact));
    assert_eq!(s.attention_rank(), 1, "compact is informational");
}

#[test]
fn cwd_changes() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("CwdChanged"), 1, &th());
    assert_eq!(s.shell_cwd.as_deref(), Some("/tmp"));
    assert_eq!(
        s.cwd.as_deref(),
        Some("/home/user/cc-sidebar-sandbox/charlie"),
        "project dir unchanged"
    );
}

#[test]
fn store_drops_internal_events_and_orders() {
    let mut st = Store::default();
    assert!(!st.apply(&fx("SessionEnd.internal-call"), 1));
    let unknown =
        hook_from(serde_json::json!({"session_id": "p", "hook_event_name": "SomeFutureEvent"}));
    assert!(!st.apply(&unknown, 1));
    assert!(st.ordered().is_empty());

    let a = fx_with("Stop", serde_json::json!({"session_id": "a"}));
    let b = fx_with("UserPromptSubmit", serde_json::json!({"session_id": "b"}));
    let c = fx_with("PermissionRequest", serde_json::json!({"session_id": "c"}));
    st.apply(&a, 1);
    st.apply(&b, 2);
    st.apply(&c, 3);
    let ids: Vec<_> = st.ordered().iter().map(|s| s.session_id.clone()).collect();
    assert_eq!(ids, ["c", "b", "a"]);
}

#[test]
fn closed_sessions_stay_until_dismissed_or_pruned() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "b"})),
        0,
    );
    for id in ["a", "b"] {
        st.apply(
            &fx_with("UserPromptSubmit", serde_json::json!({"session_id": id})),
            0,
        );
    }
    st.apply(
        &fx_with(
            "SessionEnd.tab-closed",
            serde_json::json!({"session_id": "a"}),
        ),
        1_000,
    );
    st.tick(60 * 60_000);
    let a = st.get("a").expect("closed sessions stay");
    assert_eq!(a.status, Status::Closed);
    assert_eq!(a.closed_reason.as_deref(), Some("other"));
    assert!(!st.dismiss_session("b"), "open sessions can't be dismissed");
    assert!(st.dismiss_session("a"));
    assert!(st.get("a").is_none());

    // Process gone without SessionEnd.
    assert_eq!(st.pids(), vec![(None, 382308)]);
    st.pids_dead(&[382308], 20_000);
    let b = st.get("b").unwrap();
    assert_eq!(b.status, Status::Closed);
    assert_eq!(b.closed_reason.as_deref(), Some("lost"));
    assert!(st.pids().is_empty());
    st.tick(20_000 + st.closed_retention_ms);
    assert!(st.get("b").is_none(), "pruned after retention");
}

#[test]
fn clear_replaces_the_session() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    st.apply(
        &fx_with(
            "SessionEnd",
            serde_json::json!({"session_id": "a", "reason": "clear"}),
        ),
        1,
    );
    assert!(st.get("a").is_none());
}

#[test]
fn sessions_survive_save_and_restore() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "live"})),
        0,
    );
    st.apply(
        &fx_with(
            "PermissionDenied.auto-classifier",
            serde_json::json!({"session_id": "live"}),
        ),
        1,
    );
    let mut plain = raw("UserPromptSubmit");
    plain["session_id"] = "plain".into();
    plain["_headers"] = serde_json::json!({});
    st.apply(&hook_from(plain), 2);
    let json = serde_json::to_string(&st.snapshot()).unwrap();

    let mut st2 = Store::default();
    st2.restore(serde_json::from_str(&json).unwrap(), 100);
    let live = st2.get("live").unwrap();
    assert_eq!(
        live.status,
        Status::Working,
        "has a PID: liveness will confirm"
    );
    assert!(live.has_alert(AlertKind::Denied));
    let plain = st2.get("plain").unwrap();
    assert_eq!(
        plain.status,
        Status::Closed,
        "no PID: can't confirm, so closed until it speaks"
    );
    // Restored alert ids don't collide with new ones.
    let mut live = live.clone();
    live.apply(
        &fx_with("PreCompact", serde_json::json!({"session_id": "live"})),
        101,
        &th(),
    );
    let ids: std::collections::HashSet<_> = live.alerts.iter().map(|a| a.id).collect();
    assert_eq!(ids.len(), live.alerts.len());
}

#[test]
fn first_sentence_cases() {
    assert_eq!(first_sentence("Done. Also fixed tests."), "Done.");
    assert_eq!(
        first_sentence("\n\n## Summary\nAdded goodbye() to app.py. Then ran it."),
        "Summary"
    );
    assert_eq!(
        first_sentence("Version 1.2 is out! Great"),
        "Version 1.2 is out!"
    );
    assert_eq!(first_sentence("no punctuation here"), "no punctuation here");
    assert_eq!(
        first_sentence("`app.py` has **one** function. More."),
        "app.py has one function."
    );
}

#[test]
fn questions_and_plan_approval_need_input() {
    for tool in ["AskUserQuestion", "ExitPlanMode"] {
        let mut s = Session::new(SID, 0);
        s.apply(
            &fx_with(
                "PreToolUse",
                serde_json::json!({"session_id": SID, "tool_name": tool}),
            ),
            1,
            &th(),
        );
        assert_eq!(s.status, Status::NeedsInput, "{tool}");
        // Answered: the tool completes and work carries on.
        s.apply(
            &fx_with(
                "PostToolUse",
                serde_json::json!({"session_id": SID, "tool_name": tool}),
            ),
            2,
            &th(),
        );
        assert_eq!(s.status, Status::Working);
    }
}

#[test]
fn haiku_recap_applies_only_to_its_turn() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("Stop"), 1, &th());
    let seq = s.stop_seq;
    assert!(s.set_haiku_recap(
        seq,
        crate::recap::Recap {
            line: "Added goodbye()".into(),
            detail: Some("detail".into()),
            title: Some("Goodbye helper".into())
        }
    ));
    assert_eq!(s.recap_haiku.as_deref(), Some("Added goodbye()"));
    // Next turn: the old Haiku recap goes, and a late result for the old turn is ignored.
    s.apply(&ours("Stop"), 2, &th());
    assert_eq!(s.recap_haiku, None);
    assert!(!s.set_haiku_recap(
        seq,
        crate::recap::Recap {
            line: "stale".into(),
            detail: None,
            title: None
        }
    ));
    assert_eq!(s.recap_haiku, None);
}

#[test]
fn resuming_with_plain_claude_drops_the_old_process_identity() {
    let mut st = Store::default();
    // First run through claudes: tagged, with a PID.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    assert_eq!(st.get("a").unwrap().pid, Some(382308));
    st.apply(
        &fx_with(
            "SessionEnd.tab-closed",
            serde_json::json!({"session_id": "a"}),
        ),
        1_000,
    );
    // Resumed with plain claude: same session id, no tag/PID headers.
    let mut resumed = raw("SessionStart");
    resumed["session_id"] = "a".into();
    resumed["source"] = "resume".into();
    resumed["_headers"] = serde_json::json!({});
    st.apply(&hook_from(resumed), 2_000);
    let s = st.get("a").unwrap();
    assert_eq!(s.status, Status::Starting);
    assert_eq!(
        (s.tag.as_deref(), s.pid, s.wt_session.as_deref()),
        (None, None, None)
    );
    // So liveness has nothing stale to check, and the card stays open.
    assert!(st.pids().is_empty());
    st.pids_dead(&[382308], 40_000);
    assert_eq!(st.get("a").unwrap().status, Status::Starting);
    // Later events without headers don't clear anything either way.
    st.apply(
        &fx_with(
            "Stop",
            serde_json::json!({"session_id": "a", "_headers": {}}),
        ),
        50_000,
    );
    assert_eq!(st.get("a").unwrap().pid, None);
}

#[test]
fn resuming_with_claudes_takes_the_new_identity() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    st.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": "a", "source": "resume",
            "_headers": {"x-cc-tag": "new123", "x-cc-pid": "999", "x-wt-session": "wt-new"}}),
        ),
        1_000,
    );
    let s = st.get("a").unwrap();
    assert_eq!(
        (s.tag.as_deref(), s.pid, s.wt_session.as_deref()),
        (Some("new123"), Some(999), Some("wt-new"))
    );
}

fn with_project(st: &mut Store, id: &str, root: &str, now: u64) {
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": id})),
        now,
    );
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": id})),
        now,
    );
    let name = root.rsplit('/').next().unwrap().to_string();
    st.set_project(
        id,
        crate::project::Project {
            root: root.into(),
            name,
            color: "#123456".into(),
            emoji: None,
            label: None,
            worktree: None,
            branch: Some("main".into()),
        },
    );
}

#[test]
fn projects_are_tracked_in_order_and_persist() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    with_project(&mut st, "b", "/p/bravo", 1);
    with_project(&mut st, "c", "/p/alpha", 2);
    let roots: Vec<_> = st
        .projects()
        .iter()
        .map(|t| t.project.root.clone())
        .collect();
    assert_eq!(
        roots,
        ["/p/alpha", "/p/bravo"],
        "one entry per project, in order first seen"
    );
    assert_eq!(
        st.projects()[0].project.branch,
        None,
        "branch belongs to sessions"
    );

    assert!(st.move_project("/p/bravo", 0));
    let roots: Vec<_> = st
        .projects()
        .iter()
        .map(|t| t.project.root.clone())
        .collect();
    assert_eq!(roots, ["/p/bravo", "/p/alpha"]);

    let json = serde_json::to_string(st.projects()).unwrap();
    let mut st2 = Store::default();
    st2.restore_projects(serde_json::from_str(&json).unwrap());
    assert_eq!(st2.projects(), st.projects());
}

#[test]
fn close_project_only_when_no_open_sessions() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    with_project(&mut st, "b", "/p/alpha", 0);
    assert!(st.close_project("/p/alpha").is_err(), "open sessions");
    for id in ["a", "b"] {
        st.apply(
            &fx_with("SessionEnd", serde_json::json!({"session_id": id})),
            10,
        );
    }
    st.toggle_star("a");
    assert!(st.close_project("/p/alpha").is_ok());
    assert!(st.projects().is_empty());
    assert!(
        st.get("a").is_none() && st.get("b").is_none(),
        "closed sessions go too, starred included"
    );
    // A new session there tracks it again, at the bottom.
    with_project(&mut st, "z", "/p/zulu", 20);
    with_project(&mut st, "c", "/p/alpha", 21);
    assert_eq!(st.projects().last().unwrap().project.root, "/p/alpha");
}

#[test]
fn starred_sessions_are_never_pruned() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    with_project(&mut st, "b", "/p/alpha", 0);
    for id in ["a", "b"] {
        st.apply(
            &fx_with("SessionEnd", serde_json::json!({"session_id": id})),
            10,
        );
    }
    assert!(st.toggle_star("a"));
    st.tick(10 + st.closed_retention_ms);
    assert!(st.get("a").is_some_and(|s| s.starred));
    assert!(st.get("b").is_none());
}

#[test]
fn ask_user_question_is_not_a_permission_decision() {
    let q = serde_json::json!({"questions": [{"question": "Which database should we use?", "header": "DB",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]}]});
    let mut s = Session::new(SID, 0);
    s.apply(
        &fx_with(
            "PreToolUse",
            serde_json::json!({"session_id": SID, "tool_name": "AskUserQuestion", "tool_input": q}),
        ),
        1,
        &th(),
    );
    s.apply(
        &fx_with(
            "PermissionRequest",
            serde_json::json!({"session_id": SID, "tool_name": "AskUserQuestion", "tool_input": q}),
        ),
        2,
        &th(),
    );
    assert_eq!(s.status, Status::NeedsInput);
    assert!(s.pending_permission.is_none(), "no Allow/Deny");
    assert_eq!(
        s.current_tool.as_deref(),
        Some("Asks: Which database should we use?")
    );
    // Claude Code's "needs your permission" notification follows ~6 s later: still a question.
    s.apply(
        &fx_with("Notification", serde_json::json!({"session_id": SID})),
        8_000,
        &th(),
    );
    assert_eq!(s.status, Status::NeedsInput);
}

#[test]
fn unused_new_sessions_disappear_when_closed() {
    let mut st = Store::default();
    // Started and closed without a prompt: gone.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "new"})),
        0,
    );
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "new"})),
        1,
    );
    assert!(st.get("new").is_none());
    // Prompted once: kept (resumable), and named by its first prompt until a title exists.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "used"})),
        0,
    );
    st.apply(
        &fx_with(
            "UserPromptSubmit",
            serde_json::json!({"session_id": "used"}),
        ),
        1,
    );
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "used"})),
        2,
    );
    let used = st.get("used").unwrap();
    assert!(
        used.first_prompt
            .as_deref()
            .unwrap()
            .starts_with("Do these steps in order")
    );
    // Resumed and closed without a new prompt: still has history, kept.
    st.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": "old", "source": "resume"}),
        ),
        0,
    );
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "old"})),
        1,
    );
    assert!(st.get("old").is_some());
    // Starred: kept. Process gone without SessionEnd: same rule.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "star"})),
        0,
    );
    st.toggle_star("star");
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "star"})),
        1,
    );
    assert!(st.get("star").is_some());
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "crash"})),
        0,
    );
    st.pids_dead(&[382308], 5);
    assert!(st.get("crash").is_none());
    assert!(st.get("star").is_some() && st.get("used").is_some() && st.get("old").is_some());
}

#[test]
fn names_from_transcripts() {
    use crate::session::transcript_names;
    let t = [
        r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>"}}"#,
        r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Add a shout() helper to app.py"}]}}"#,
        r#"{"type":"ai-title","aiTitle":"Shout helper"}"#,
        r#"{"type":"user","message":{"content":"and a docstring"}}"#,
        r#"{"type":"ai-title","aiTitle":"Shout helper with docs"}"#,
        "not json",
    ]
    .join("\n");
    assert_eq!(
        transcript_names(&t),
        (
            Some("Shout helper with docs".into()),
            Some("Add a shout() helper to app.py".into())
        )
    );
    let custom = format!(
        "{t}\n{}",
        r#"{"type":"custom-title","customTitle":"My name"}"#
    );
    assert_eq!(transcript_names(&custom).0.as_deref(), Some("My name"));
    assert_eq!(transcript_names(""), (None, None));
}

#[test]
fn instant_recap_keeps_more_of_the_message() {
    let msg = "## Done\n\nI added **shout()** to `app.py`.\n\n- It upper-cases\n- It adds !\n\n```py\ncode\n```\nNext: tests.";
    assert_eq!(
        recap_text(msg),
        "Done I added shout() to app.py. - It upper-cases - It adds ! code Next: tests."
    );
    assert!(recap_text(&"word ".repeat(200)).chars().count() <= 400);
}

#[test]
fn renaming_sessions() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "a"})),
        1,
    );
    // Open: named in the sidebar, nothing to write yet.
    assert!(st.rename("a", "  Login bug  ").is_none());
    assert_eq!(
        st.get("a").unwrap().user_title.as_deref(),
        Some("Login bug")
    );
    assert!(st.unsynced_titles().is_empty());
    // Closed: now it's due for the transcript, once.
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "a"})),
        2,
    );
    assert_eq!(st.unsynced_titles().len(), 1);
    st.mark_title_synced("a");
    assert!(st.unsynced_titles().is_empty());
    // Renaming a closed session asks for an immediate write; "" clears the name.
    assert!(st.rename("a", "Login fix").is_some());
    assert!(st.rename("a", "").is_none());
    assert_eq!(st.get("a").unwrap().user_title, None);
}

#[test]
fn title_records_append_on_a_fresh_line() {
    let dir = std::env::temp_dir().join(format!("cc-sidebar-title-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.jsonl");
    std::fs::write(&f, "{\"type\":\"user\"}").unwrap(); // no trailing newline
    crate::server::append_title(&f, "sid", "My name").unwrap();
    let text = std::fs::read_to_string(&f).unwrap();
    let (title, _) = crate::session::transcript_names(&text);
    assert_eq!(title.as_deref(), Some("My name"));
    assert_eq!(text.lines().count(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}
