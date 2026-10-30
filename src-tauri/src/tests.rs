//! State machine tests driven by real hook payloads captured in dev/fixtures/ (M1 spikes).
use crate::events::{Event, Hook, HookMeta};
use crate::session::{
    AlertKind, Session, Status, Thresholds, first_sentence, looks_awaited, recap_text,
};
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

    // Resumed: any event revives it, ready for a prompt (not "new").
    s.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": SID, "source": "resume"}),
        ),
        8,
        &th(),
    );
    assert_eq!(s.status, Status::Idle);
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
    // A background agent counts as an agent, not a background command.
    s.apply(&ours("Stop.background-tasks"), 1, &th());
    assert_eq!(s.status, Status::Idle);
    assert_eq!((s.subagents_running, s.background_running), (1, 0));
    assert_eq!(s.background[0].description, "Count lines in README.md");
    s.apply(&ours("Stop.background-shell"), 2, &th());
    assert_eq!((s.subagents_running, s.background_running), (0, 1));
    let b = &s.background[0];
    assert_eq!(
        (
            b.id.as_str(),
            b.agent,
            b.description.as_str(),
            b.command.as_str(),
            b.awaited
        ),
        ("bxyho0k31", false, "sleep 90", "sleep 90", false)
    );
    // Its output file sits next to the scratchpad the hooks report.
    assert!(
        s.tasks_dir
            .as_deref()
            .is_some_and(|d| d.ends_with("/d60030c3-51ca-429b-bf09-a8efbfa2013a/tasks")),
        "{:?}",
        s.tasks_dir
    );
    s.apply(&ours("Stop"), 3, &th());
    assert_eq!((s.subagents_running, s.background_running), (0, 0));
    assert!(s.background.is_empty());
}

#[test]
fn background_agents_work_while_ready() {
    // The turn ends with an agent still running in the background.
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    s.apply(&ours("Stop.background-tasks"), 2, &th());
    assert_eq!(s.status, Status::Idle);
    let agent = |name: &str| {
        fx_with(
            name,
            serde_json::json!({ "session_id": SID, "agent_id": "a8c13db353cf1070d" }),
        )
    };
    // Its tool calls aren't a turn: still ready, same timer, no activity line.
    s.apply(&agent("PreToolUse"), 3, &th());
    s.apply(&agent("PostToolUse"), 4, &th());
    assert_eq!((s.status, s.status_since), (Status::Idle, 2));
    assert_eq!(s.current_tool, None);
    assert!(s.trail.is_empty());

    // Its permission request needs the user; once it carries on, back to ready.
    s.apply(&agent("PermissionRequest"), 5, &th());
    assert_eq!(s.status, Status::NeedsPermission);
    assert!(s.pending_permission.is_some());
    s.apply(&agent("PostToolUse"), 6, &th());
    assert_eq!((s.status, s.status_since), (Status::Idle, 2));
    assert!(s.pending_permission.is_none());

    // It finishing still counts down and updates the list.
    s.apply(
        &fx_with(
            "SubagentStop",
            serde_json::json!({ "session_id": SID, "agent_id": "a8c13db353cf1070d",
                "agent_type": "general-purpose", "background_tasks": [] }),
        ),
        7,
        &th(),
    );
    assert_eq!(s.subagents_running, 0);
    assert!(s.background.is_empty());

    // Left "working" by an agent's tool call before they were told apart: ready again.
    s.status = Status::Working;
    s.current_tool = Some("Reading".into());
    s.background = vec![];
    s.apply(&ours("Stop.background-tasks"), 7, &th());
    s.status = Status::Working;
    s.apply(&agent("PreToolUse"), 8, &th());
    assert_eq!((s.status, s.status_since), (Status::Idle, 7));
    assert_eq!(s.current_tool, None);

    // A foreground agent's tool calls are part of the turn as before.
    s.apply(&ours("UserPromptSubmit"), 9, &th());
    let foreground = fx_with(
        "PreToolUse",
        serde_json::json!({ "session_id": SID, "agent_id": "a0000000000000001" }),
    );
    s.apply(&foreground, 10, &th());
    assert_eq!(s.status, Status::Working);
    assert!(s.current_tool.is_some());
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
    assert!(s.has_alert(AlertKind::Compact));
    assert_eq!(s.attention_rank(), 1, "compact is informational");
    s.apply(&ours("SessionStart.compact"), 3, &th());
    assert_eq!(s.status, Status::Working);
    assert!(
        !s.has_alert(AlertKind::Compact),
        "cleared once compaction is done"
    );
}

#[test]
fn idle_compaction_isnt_a_full_context() {
    let auto = || {
        fx_with(
            "PreCompact",
            serde_json::json!({"session_id": SID, "trigger": "auto"}),
        )
    };
    let text = |s: &Session| {
        s.alerts
            .iter()
            .find(|a| a.kind == AlertKind::Compact)
            .unwrap()
            .text
            .clone()
    };
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    s.apply(&auto(), 2, &th());
    assert_eq!(text(&s), "Context full: compacting");
    s.apply(&ours("Stop"), 3, &th());
    s.apply(&auto(), 4, &th());
    assert_eq!(text(&s), "Compacting while idle, before the cache expires");
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
fn project_dir_ignores_cd_into_subfolders() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    // Claude cd'd into a nested repo: the payload cwd follows, the transcript path doesn't.
    let sub = "/home/user/cc-sidebar-sandbox/charlie/api/src";
    s.apply(
        &fx_with("Stop", serde_json::json!({ "session_id": SID, "cwd": sub })),
        2,
        &th(),
    );
    assert_eq!(
        s.cwd.as_deref(),
        Some("/home/user/cc-sidebar-sandbox/charlie")
    );
    assert_eq!(
        crate::session::start_dir(
            sub,
            Some("/x/.claude/projects/-home-user-dev-my-app/1.jsonl")
        ),
        None,
        "no matching ancestor"
    );
    assert_eq!(
        crate::session::start_dir(
            "/home/user/dev/my.app/web",
            Some("/x/.claude/projects/-home-user-dev-my-app/1.jsonl")
        )
        .as_deref(),
        Some("/home/user/dev/my.app")
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
    assert!(!st.dismiss_session("a"), "only removed ones can be deleted");
    st.set_removed("a", true);
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
    assert!(st.get("b").is_some(), "never removed: kept");
    st.set_removed("b", true);
    st.tick(20_000 + st.closed_retention_ms);
    assert!(st.get("b").is_none(), "removed: pruned after retention");
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
            title: Some("Goodbye helper".into()),
            name: None,
            waiting: None,
            background: None,
            labels: vec![],
        },
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
            title: None,
            name: None,
            waiting: None,
            background: None,
            labels: vec![],
        },
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
    assert_eq!(s.status, Status::Idle);
    assert_eq!(
        (s.tag.as_deref(), s.pid, s.wt_session.as_deref()),
        (None, None, None)
    );
    // So liveness has nothing stale to check, and the card stays open.
    assert!(st.pids().is_empty());
    st.pids_dead(&[382308], 40_000);
    assert_eq!(st.get("a").unwrap().status, Status::Idle);
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
            repos: Default::default(),
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
    assert!(st.close_project("/p/alpha").is_ok());
    assert!(st.projects().is_empty());
    assert!(
        st.get("a").is_none() && st.get("b").is_none(),
        "its closed sessions go too"
    );
    // A new session there tracks it again, at the bottom.
    with_project(&mut st, "z", "/p/zulu", 20);
    with_project(&mut st, "c", "/p/alpha", 21);
    assert_eq!(st.projects().last().unwrap().project.root, "/p/alpha");
}

#[test]
fn only_removed_sessions_are_pruned() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    with_project(&mut st, "b", "/p/alpha", 0);
    for id in ["a", "b"] {
        st.apply(
            &fx_with(
                "SessionEnd.tab-closed",
                serde_json::json!({"session_id": id}),
            ),
            10,
        );
    }
    assert!(st.set_removed("b", true));
    assert!(!st.dismiss_session("a"), "not removed: can't be deleted");
    st.tick(10 + st.closed_retention_ms);
    assert!(st.get("a").is_some(), "kept until the user removes it");
    assert!(st.get("b").is_none());
}

#[test]
fn exit_removes_and_resuming_brings_back() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    st.apply(
        &fx_with(
            "SessionEnd",
            serde_json::json!({"session_id": "a", "reason": "prompt_input_exit"}),
        ),
        1,
    );
    assert!(
        st.get("a").unwrap().is_removed(),
        "/exit means done with it"
    );
    st.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": "a", "source": "resume"}),
        ),
        2,
    );
    assert!(!st.get("a").unwrap().is_removed());
    // A tab closed or a process gone: still in the list.
    st.apply(
        &fx_with(
            "SessionEnd.tab-closed",
            serde_json::json!({"session_id": "a"}),
        ),
        3,
    );
    assert!(!st.get("a").unwrap().is_removed());
}

#[test]
fn saved_closed_sessions_migrate_to_removed_unless_starred() {
    let mk = |starred: bool| {
        let mut s = Session::new(SID, 0);
        s.apply(&ours("SessionEnd.tab-closed"), 1, &th());
        let mut v = serde_json::to_value(&s).unwrap();
        v.as_object_mut().unwrap().remove("removed");
        v["starred"] = starred.into();
        serde_json::from_value::<Session>(v).unwrap().restored()
    };
    assert!(mk(false).is_removed());
    assert!(!mk(true).is_removed());
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
fn idle_prompt_leaves_a_finished_turn_ready() {
    let mut s = Session::new(SID, 0);
    s.apply(
        &fx_with("Stop", serde_json::json!({"session_id": SID})),
        1_000,
        &th(),
    );
    assert_eq!(s.status, Status::Idle);
    // A minute later Claude Code says the prompt has been idle: still ready, not "your turn".
    s.apply(
        &fx_with(
            "Notification",
            serde_json::json!({"session_id": SID, "notification_type": "idle_prompt",
                "message": "Claude is waiting for your input"}),
        ),
        61_000,
        &th(),
    );
    assert_eq!(s.status, Status::Idle);
    assert_eq!(s.status_since, 1_000);
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
    // Process gone without SessionEnd: same rule.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "crash"})),
        0,
    );
    st.pids_dead(&[382308], 5);
    assert!(st.get("crash").is_none());
    assert!(st.get("used").is_some() && st.get("old").is_some());
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

#[test]
fn shelved_projects_persist_and_come_back_for_new_sessions() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    assert!(st.set_shelved("/p/alpha", true));
    assert!(!st.set_shelved("/p/alpha", true), "no change");
    assert!(st.set_session_order("/p/alpha", vec!["b".into(), "a".into()]));
    let json = serde_json::to_string(st.projects()).unwrap();
    let mut st2 = Store::default();
    st2.restore_projects(serde_json::from_str(&json).unwrap());
    assert!(st2.projects()[0].shelved);
    assert_eq!(st2.projects()[0].session_order, ["b", "a"]);
    // Old projects.json entries have no flag.
    let old: Vec<crate::store::Tracked> = serde_json::from_str(
        r##"[{"project":{"root":"/p/x","name":"x","color":"#000","emoji":null,"label":null,"worktree":null,"branch":null},"added_at":1}]"##,
    )
    .unwrap();
    assert!(!old[0].shelved);

    // Activity in an existing session leaves it in the drawer; a new session takes it out.
    with_project(&mut st, "a", "/p/alpha", 1);
    assert!(st.projects()[0].shelved);
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "b"})),
        2,
    );
    let p = st.get("a").unwrap().project.clone().unwrap();
    st.set_project("b", p);
    assert!(!st.projects()[0].shelved);
}

fn haiku(waiting: bool) -> crate::recap::Recap {
    crate::recap::Recap {
        line: "recap".into(),
        detail: None,
        title: None,
        name: None,
        waiting: Some(waiting),
        background: None,
        labels: vec![],
    }
}

#[test]
fn jobs_claude_waits_on() {
    // A background agent is always a job; the status stays "ready".
    let mut s = Session::new(SID, 0);
    s.apply(&ours("Stop.background-tasks"), 1, &th());
    assert_eq!(s.status, Status::Idle);
    assert!(s.background[0].awaited);

    // A plain background command is a job once Haiku says Claude will act on it.
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    s.apply(&ours("Stop.background-shell"), 2, &th());
    assert!(
        !s.background[0].awaited,
        "\"sleep 90\" doesn't look like a watcher"
    );
    assert!(s.set_haiku_recap(s.stop_seq, haiku(true)));
    assert!(s.background[0].awaited);
    assert_eq!(s.status, Status::Idle);

    // The user chats in between: still a job, and no alert when that turn ends.
    s.apply(&ours("UserPromptSubmit"), 3, &th());
    s.apply(&ours("Stop.background-shell"), 4, &th());
    assert!(!s.has_alert(AlertKind::Finished));
    s.set_haiku_recap(s.stop_seq, haiku(true));

    // The job reports back (an injected prompt), Claude deals with it and stops.
    // Recorded: the prompt Claude Code injects when a background command finishes.
    s.apply(&ours("UserPromptSubmit.task-notification"), 5, &th());
    s.apply(&ours("Stop"), 6, &th());
    assert_eq!(s.status, Status::Idle);
    assert!(s.has_alert(AlertKind::Finished), "ready for you right away");
    assert!(s.pending_jobs.is_empty());
    s.apply(&ours("UserPromptSubmit"), 7, &th());
    assert!(!s.has_alert(AlertKind::Finished));
}

#[test]
fn jobs_finishing_during_the_users_turn_dont_alert() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("Stop.background-tasks"), 1, &th());
    s.apply(&ours("UserPromptSubmit"), 2, &th());
    s.apply(&ours("Stop"), 3, &th());
    assert!(!s.has_alert(AlertKind::Finished));
}

#[test]
fn haiku_can_say_background_commands_are_not_jobs() {
    let mut s = Session::new(SID, 0);
    s.apply(
        &fx_with(
            "Stop",
            serde_json::json!({"session_id": SID, "background_tasks": [
                {"id": "x", "type": "shell", "status": "running",
                 "description": "Watch the logs until I stop it"}]}),
        ),
        1,
        &th(),
    );
    assert!(s.background[0].awaited, "guessed from the description");
    s.set_haiku_recap(s.stop_seq, haiku(false));
    assert!(!s.background[0].awaited);
    assert!(s.pending_jobs.is_empty());
}

#[test]
fn awaited_background_commands() {
    assert!(looks_awaited(
        "Watch the nightly ML job until it finishes",
        ""
    ));
    assert!(looks_awaited("Poll the deploy status", ""));
    assert!(!looks_awaited("Start the dev server npm run dev", ""));
    assert!(!looks_awaited("sleep 90 sleep 90", ""));
    assert!(!looks_awaited("tail -f app.log", ""));
    assert!(looks_awaited("Watch the CN, PL and frontend deploys", ""));
    assert!(crate::session::looks_service(
        "Start reports API with read-only groups",
        ""
    ));
    assert!(!crate::session::looks_service("Check capital gains", ""));
    // Seen live: a pipeline watcher whose script calls REST APIs (`api-version=`): the
    // command's words don't make it a service; its description does the talking.
    let (d, c) = (
        "Follow the optimisation through dispatch, ML job and ingestion",
        "az rest --uri \"https://management.azure.com/x?api-version=2023-01-01\"",
    );
    assert!(!crate::session::looks_service(d, c));
    assert!(looks_awaited(d, c));
    // A dev server is still recognised from its command.
    assert!(crate::session::looks_service(
        "Start the app",
        "cd web && npm run dev"
    ));
}

#[test]
fn haiku_cannot_make_servers_jobs() {
    // Seen live: two servers started in the background, and Haiku said the session was
    // waiting on background work. Only real jobs follow Haiku.
    let mut s = Session::new(SID, 0);
    s.apply(
        &fx_with(
            "Stop",
            serde_json::json!({"session_id": SID, "background_tasks": [
                {"id": "1", "type": "shell", "status": "running",
                 "description": "Run the API backend against dev DB in background"},
                {"id": "2", "type": "shell", "status": "running",
                 "description": "Run the Vite frontend dev server in background"},
                {"id": "3", "type": "shell", "status": "running",
                 "description": "Run the nightly import"}]}),
        ),
        1,
        &th(),
    );
    s.set_haiku_recap(s.stop_seq, haiku(true));
    let awaited: Vec<bool> = s.background.iter().map(|b| b.awaited).collect();
    assert_eq!(awaited, [false, false, true]);
}

fn tmux_session(st: &mut Store, id: &str, now: u64) {
    st.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": id, "source": "resume",
                "_headers": {"x-cc-pid": "4242", "x-cc-tag": "t1", "x-cc-tmux": "cc-t1"}}),
        ),
        now,
    );
}

#[test]
fn resumed_sessions_are_ready_not_new() {
    let mut st = Store::default();
    tmux_session(&mut st, "r", 0);
    assert_eq!(st.get("r").unwrap().status, Status::Idle);
    assert_eq!(st.get("r").unwrap().tmux.as_deref(), Some("cc-t1"));
}

#[test]
fn idle_sessions_nothing_shows_are_ended() {
    let attached = |on: bool| std::collections::HashMap::from([("cc-t1".to_string(), on)]);
    let mut st = Store::default();
    tmux_session(&mut st, "a", 0);
    let distro = st.get("a").unwrap().distro.clone();

    // Shown: kept.
    st.set_attached(&distro, &attached(true), 1_000);
    assert!(st.take_idle_unwatched(60_000).is_empty());
    // Just closed: not before the grace period (a click may be about to attach a view).
    st.set_attached(&distro, &attached(false), 61_000);
    assert!(st.take_idle_unwatched(62_000).is_empty());
    assert_eq!(st.take_idle_unwatched(67_000), vec![(distro.clone(), 4242)]);
    assert!(st.take_idle_unwatched(70_000).is_empty(), "only once");
    // Its SessionEnd reads as idle, not "tab closed".
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "a"})),
        71_000,
    );
    assert_eq!(st.get("a").unwrap().closed_reason.as_deref(), Some("idle"));
}

#[test]
fn closing_the_view_of_an_unprompted_session_ends_it_at_once() {
    let attached = |on: bool| std::collections::HashMap::from([("cc-t1".to_string(), on)]);
    let mut st = Store::default();
    st.apply(
        &fx_with(
            "SessionStart",
            serde_json::json!({"session_id": "n", "source": "startup",
                "_headers": {"x-cc-pid": "4242", "x-cc-tag": "t1", "x-cc-tmux": "cc-t1"}}),
        ),
        1_000,
    );
    let d = st.get("n").unwrap().distro.clone();
    // Not yet attached right after launch: the grace still applies.
    st.set_attached(&d, &attached(false), 1_500);
    assert!(st.take_idle_unwatched(2_000).is_empty());
    st.set_attached(&d, &attached(true), 2_500);
    // The user closes its view: ended on the next check, no grace.
    st.view_closed("cc-t1");
    st.set_attached(&d, &attached(false), 3_000);
    assert_eq!(st.take_idle_unwatched(3_000).len(), 1);
    // Its SessionEnd removes the card.
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "n"})),
        3_500,
    );
    assert!(st.get("n").is_none());

    // A prompted session keeps the grace (it stays listed, resumable).
    let mut st = Store::default();
    tmux_session(&mut st, "a", 0);
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "a"})),
        1,
    );
    st.apply(&fx_with("Stop", serde_json::json!({"session_id": "a"})), 2);
    st.view_closed("cc-t1");
    st.set_attached(&d, &attached(false), 10_000);
    assert!(st.take_idle_unwatched(10_000).is_empty());
}

#[test]
fn sessions_with_something_to_do_stay() {
    let attached = std::collections::HashMap::from([("cc-t1".to_string(), false)]);
    // Working.
    let mut st = Store::default();
    tmux_session(&mut st, "w", 0);
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "w"})),
        1,
    );
    let d = st.get("w").unwrap().distro.clone();
    st.set_attached(&d, &attached, 2);
    assert!(st.take_idle_unwatched(100_000).is_empty());
    // A background command (e.g. a dev server) still running.
    let mut st = Store::default();
    tmux_session(&mut st, "b", 0);
    st.apply(
        &fx_with(
            "Stop.background-shell",
            serde_json::json!({"session_id": "b"}),
        ),
        1,
    );
    st.set_attached(&d, &attached, 2);
    assert!(st.take_idle_unwatched(100_000).is_empty());
}

#[test]
fn ready_timer_survives_idle_end_and_resume() {
    let attached = std::collections::HashMap::from([("cc-t1".to_string(), false)]);
    let mut st = Store::default();
    tmux_session(&mut st, "a", 0);
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "a"})),
        1_000,
    );
    st.apply(
        &fx_with("Stop", serde_json::json!({"session_id": "a"})),
        2_000,
    );
    assert_eq!(st.get("a").unwrap().status_since, 2_000);
    // Nothing shows it: ended for sitting idle. The timer still counts from the Stop.
    let d = st.get("a").unwrap().distro.clone();
    st.set_attached(&d, &attached, 3_000);
    assert_eq!(st.take_idle_unwatched(10_000).len(), 1);
    st.apply(
        &fx_with("SessionEnd", serde_json::json!({"session_id": "a"})),
        11_000,
    );
    assert_eq!(st.get("a").unwrap().status_since, 2_000);
    // Opened again (resumed) without a prompt: same.
    tmux_session(&mut st, "a", 50_000);
    let s = st.get("a").unwrap();
    assert_eq!((s.status, s.status_since), (Status::Idle, 2_000));
}

#[test]
fn timers_survive_a_restart() {
    // Ready when the machine went down; found gone at the next start: same timer.
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "a"})),
        1_000,
    );
    st.apply(
        &fx_with("Stop", serde_json::json!({"session_id": "a"})),
        2_000,
    );
    st.apply(
        &fx_with(
            "Notification",
            serde_json::json!({"session_id": "a", "notification_type": "idle_prompt"}),
        ),
        62_000,
    );
    // Working when it went: counts from its last sign of life.
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "b"})),
        0,
    );
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "b"})),
        5_000,
    );
    st.apply(
        &fx_with("PostToolUse", serde_json::json!({"session_id": "b"})),
        7_000,
    );
    let pids: Vec<u32> = st.pids().into_iter().map(|(_, p)| p).collect();
    st.pids_dead(&pids, 9_000_000);
    let a = st.get("a").unwrap();
    assert_eq!((a.status, a.status_since), (Status::Closed, 2_000));
    assert_eq!(st.get("b").unwrap().status_since, 7_000);

    // Saved by a build that timed it from the restart: repaired on load.
    let mut old = st.get("b").unwrap().clone();
    old.status_since = 9_000_000;
    assert_eq!(old.restored().status_since, 7_000);
}

#[test]
fn opening_a_session_clears_job_finished() {
    let mut st = Store::default();
    st.apply(&ours("Stop.background-tasks"), 1);
    st.apply(&ours("UserPromptSubmit.task-notification"), 2);
    st.apply(&ours("Stop"), 3);
    assert!(st.get(SID).unwrap().has_alert(AlertKind::Finished));
    assert_eq!(
        st.get(SID).unwrap().last_prompt,
        None,
        "a task notification isn't the user's prompt"
    );
    assert!(st.seen(SID));
    assert!(!st.get(SID).unwrap().has_alert(AlertKind::Finished));
    assert!(!st.seen(SID), "nothing left to clear");
}

#[test]
fn haiku_labels_each_background_task() {
    // Seen live: "Start reports API …" matched no service word then, and the session-wide
    // verdict said yes. Per-task labels from Haiku decide instead.
    let mut s = Session::new(SID, 0);
    s.apply(
        &fx_with(
            "Stop",
            serde_json::json!({"session_id": SID, "background_tasks": [
                {"id": "1", "type": "shell", "status": "running",
                 "description": "Start reports API with read-only groups"},
                {"id": "2", "type": "shell", "status": "running",
                 "description": "Resume the retest after approval"}]}),
        ),
        1,
        &th(),
    );
    let mut r = haiku(true);
    r.labels = vec![
        ("Start reports API with read-only groups".into(), false),
        ("Resume the retest after approval".into(), true),
    ];
    s.set_haiku_recap(s.stop_seq, r);
    let awaited: Vec<bool> = s.background.iter().map(|b| b.awaited).collect();
    assert_eq!(awaited, [false, true]);
    // The labels outlive a refresh of the list (a subagent finishing mid-turn).
    s.apply(
        &fx_with(
            "SubagentStop",
            serde_json::json!({"session_id": SID, "agent_type": "", "background_tasks": [
                {"id": "1", "type": "shell", "status": "running",
                 "description": "Start reports API with read-only groups"}]}),
        ),
        2,
        &th(),
    );
    assert!(!s.background[0].awaited);
}

#[test]
fn last_answer_as_plain_text_with_its_question() {
    let msg = "**Done.** Pushed `main` to origin.\n\n## Changes\n- fixed the parser\n  - and its tests\n\n```sh\ngit push\n```\n\nWant me to open a PR? Or merge it.\nShall I commit and push it?";
    let a = crate::session::answer_text(msg);
    assert_eq!(
        a,
        "Done. Pushed main to origin.\nChanges\n• fixed the parser\n  • and its tests\ngit push\nWant me to open a PR? Or merge it.\nShall I commit and push it?"
    );
    assert_eq!(
        crate::session::ending_question(msg).as_deref(),
        Some("Shall I commit and push it?")
    );
    assert_eq!(
        crate::session::ending_question("Done. Anything else? I fixed it.").as_deref(),
        None
    );
    assert_eq!(
        crate::session::ending_question("I fixed it. Want me to push?").as_deref(),
        Some("Want me to push?")
    );
}

#[test]
fn last_assistant_text_from_transcript() {
    let t = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Old answer."}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"New answer. Push it?"},{"type":"tool_use","name":"Bash"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"thanks"}}"#,
        "\n\u{0}\u{0}\n",
    );
    assert_eq!(
        crate::session::last_assistant_text(t).as_deref(),
        Some("New answer. Push it?")
    );
    assert_eq!(crate::session::last_assistant_text(""), None);
}

#[test]
fn titles_stick_but_a_stored_name_wins() {
    let mut s = Session::new(SID, 0);
    let recap = |title: Option<&str>, name: Option<&str>| crate::recap::Recap {
        line: "x".into(),
        detail: None,
        title: title.map(Into::into),
        name: name.map(Into::into),
        waiting: None,
        background: None,
        labels: vec![],
    };
    s.apply(&ours("Stop"), 1, &th());
    s.set_haiku_recap(s.stop_seq, recap(Some("Seed the schedule"), None));
    s.apply(&ours("Stop"), 2, &th());
    s.set_haiku_recap(s.stop_seq, recap(Some("Debug a missing run"), None));
    assert_eq!(
        s.title.as_deref(),
        Some("Seed the schedule"),
        "Haiku names it once"
    );
    s.apply(&ours("Stop"), 3, &th());
    s.set_haiku_recap(s.stop_seq, recap(None, Some("Renamed in Claude Code")));
    assert_eq!(s.title.as_deref(), Some("Renamed in Claude Code"));
}

#[test]
fn minimised_cards_persist() {
    let mut st = Store::default();
    with_project(&mut st, "a", "/p/alpha", 0);
    assert!(st.set_minimized(&["a".into()], true));
    assert!(!st.set_minimized(&["a".into()], true), "no change");
    let json = serde_json::to_string(&st.snapshot()).unwrap();
    let mut st2 = Store::default();
    st2.restore(serde_json::from_str(&json).unwrap(), 1);
    assert!(st2.get("a").unwrap().minimized);
}

#[test]
fn pr_alerts_on_failing_checks_changes_requested_and_merge() {
    use crate::prs::{Checks, Found, Host, Review, State, Status};
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "p"})),
        0,
    );
    let found = Found {
        host: Host::Github,
        number: 7,
        url: Some("https://github.com/o/r/pull/7".into()),
        org: None,
        repo: Some("r".into()),
    };
    assert!(st.add_prs("p", vec![found.clone()], true));
    assert!(!st.add_prs("p", vec![found], false), "already known");
    let due = st.prs_due(1_000);
    assert_eq!(due.len(), 1);
    let pr = due[0].3.clone();
    let status = |checks, review, state| Status {
        state: Some(state),
        checks: Some(checks),
        review: Some(review),
        title: Some("Fix login".into()),
        url: None,
    };
    // A failed lookup, then the first one that works: no alert, even if checks are already
    // failing.
    st.set_pr_status("p", &pr, None, 500);
    st.set_pr_status(
        "p",
        &pr,
        Some(status(Checks::Failing, Review::Required, State::Open)),
        1_000,
    );
    assert!(st.get("p").unwrap().alerts.is_empty());
    assert!(st.prs_due(2_000).is_empty(), "not due again yet");
    let last = |st: &Store| st.get("p").unwrap().alerts.last().map(|a| a.text.clone());
    let set = |st: &mut Store, c, r, s, at| st.set_pr_status("p", &pr, Some(status(c, r, s)), at);
    set(
        &mut st,
        Checks::Passing,
        Review::Required,
        State::Open,
        400_000,
    );
    assert_eq!(last(&st), None);
    set(
        &mut st,
        Checks::Failing,
        Review::Required,
        State::Open,
        800_000,
    );
    assert_eq!(last(&st).as_deref(), Some("PR checks failed: #7 Fix login"));
    set(
        &mut st,
        Checks::Failing,
        Review::ChangesRequested,
        State::Open,
        900_000,
    );
    assert_eq!(
        last(&st).as_deref(),
        Some("Changes requested: #7 Fix login")
    );
    set(
        &mut st,
        Checks::Passing,
        Review::Approved,
        State::Merged,
        1_200_000,
    );
    assert_eq!(last(&st).as_deref(), Some("PR merged: #7 Fix login"));
    // Merged: no more lookups.
    assert!(st.prs_due(10_000_000).is_empty());
}

#[test]
fn azure_prs_without_org_take_the_sessions_and_merge() {
    use crate::prs::{Found, Host};
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "a"})),
        0,
    );
    let az = |n, org: Option<&str>| Found {
        host: Host::Azure,
        number: n,
        url: None,
        org: org.map(str::to_string),
        repo: None,
    };
    st.add_prs("a", vec![az(1, None)], true);
    st.add_prs(
        "a",
        vec![
            az(2, Some("https://dev.azure.com/acme")),
            az(1, Some("https://dev.azure.com/acme")),
        ],
        false,
    );
    let prs = &st.get("a").unwrap().prs;
    assert_eq!(prs.len(), 2);
    assert!(
        prs.iter()
            .all(|p| p.org.as_deref() == Some("https://dev.azure.com/acme"))
    );
}

#[test]
fn ready_timer_survives_a_closed_tab() {
    let mut st = Store::default();
    st.apply(
        &fx_with("SessionStart", serde_json::json!({"session_id": "t"})),
        0,
    );
    st.apply(
        &fx_with("UserPromptSubmit", serde_json::json!({"session_id": "t"})),
        1_000,
    );
    st.apply(
        &fx_with("Stop", serde_json::json!({"session_id": "t"})),
        2_000,
    );
    st.apply(
        &fx_with(
            "SessionEnd",
            serde_json::json!({"session_id": "t", "reason": "other"}),
        ),
        60_000,
    );
    let s = st.get("t").unwrap();
    assert_eq!(s.status, Status::Closed);
    assert_eq!(s.status_since, 2_000);
}

#[test]
fn a_turn_keeps_its_trail_and_task_list() {
    let mut s = Session::new(SID, 0);
    let pre = |tool: &str, input: serde_json::Value| {
        fx_with(
            "PreToolUse",
            serde_json::json!({"session_id": SID, "tool_name": tool, "tool_input": input}),
        )
    };
    s.apply(
        &fx_with(
            "UserPromptSubmit",
            serde_json::json!({"session_id": SID, "prompt": "fix it"}),
        ),
        1,
        &th(),
    );
    s.apply(
        &pre("Read", serde_json::json!({"file_path": "/a/src/auth.ts"})),
        2,
        &th(),
    );
    s.apply(
        &pre("Edit", serde_json::json!({"file_path": "/a/src/auth.ts"})),
        3,
        &th(),
    );
    s.apply(
        &pre("Edit", serde_json::json!({"file_path": "/a/src/auth.ts"})),
        4,
        &th(),
    );
    s.apply(
        &pre(
            "Bash",
            serde_json::json!({"command": "cd /a && CI=1 npm run test -- --watch"}),
        ),
        5,
        &th(),
    );
    s.apply(
        &pre("ToolSearch", serde_json::json!({"query": "x"})),
        6,
        &th(),
    );
    s.apply(
        &pre(
            "Bash",
            serde_json::json!({"command": "bash /tmp/x/shot.sh name"}),
        ),
        6,
        &th(),
    );
    let trail: Vec<_> = s
        .trail
        .iter()
        .map(|t| (t.label.as_str(), t.count))
        .collect();
    assert_eq!(
        trail,
        [
            ("📄 auth.ts", 1),
            ("✎ auth.ts", 2),
            ("▶ npm run", 1),
            ("▶ bash shot.sh", 1)
        ]
    );
    s.apply(
        &pre(
            "TodoWrite",
            serde_json::json!({"todos": [
                {"content": "Read", "status": "completed", "activeForm": "Reading"},
                {"content": "Fix", "status": "in_progress", "activeForm": "Fixing the bug"},
                {"content": "Test", "status": "pending", "activeForm": "Testing"}]}),
        ),
        7,
        &th(),
    );
    assert_eq!(
        s.todos,
        Some(crate::session::Todos {
            done: 1,
            total: 3,
            current: Some("Fixing the bug".into())
        })
    );
    // A background task reporting back starts a new turn that wasn't the user's.
    s.apply(
        &fx_with(
            "UserPromptSubmit",
            serde_json::json!({"session_id": SID, "prompt":
                "<task-notification>\n<task-id>b1</task-id>\n<summary>Background command \"Run tests\" completed (exit code 0)</summary>\n</task-notification>"}),
        ),
        8,
        &th(),
    );
    assert!(s.trail.is_empty());
    assert_eq!(
        s.turn_note.as_deref(),
        Some("Background command \"Run tests\" completed (exit code 0)")
    );
    assert_eq!(s.last_prompt.as_deref(), Some("fix it"));
}

#[test]
fn artifact_live_updates_are_a_background_task_not_a_job() {
    const D: &str =
        "live updates for artifact https://claude.ai/artifact/X (re-armed on session resume)";
    let mut s = Session::new(SID, 0);
    // Even when Haiku calls it a job.
    s.job_labels.insert(D.into(), true);
    s.apply(
        &fx_with(
            "Stop",
            serde_json::json!({"session_id": SID, "background_tasks": [{
                "id": "sal065lsw", "type": "shell", "status": "running", "command": "", "description": D}]}),
        ),
        1,
        &th(),
    );
    assert_eq!(s.background.len(), 1);
    assert!(!s.background[0].awaited);
}

#[test]
fn job_progress_makes_a_job_and_alerts_when_blocked() {
    use crate::session::Progress;
    let mut s = Session::new(SID, 0);
    // "sleep 90" reads as neither: not awaited until it reports progress.
    s.apply(&ours("Stop.background-shell"), 5, &th());
    assert!(!s.background[0].awaited);
    assert_eq!(s.background[0].seen_at, Some(5));
    let step = |at: &str, state: &str| Progress {
        item: Some("website".into()),
        steps: vec!["Build".into(), "Dev".into()],
        at: Some(at.into()),
        state: Some(state.into()),
        detail: Some("Dev needs approval".into()),
        ..Default::default()
    };
    assert!(s.job_output(
        "bxyho0k31",
        Some("polling".into()),
        vec![step("Build", "running")],
        Some(6),
        7
    ));
    assert!(s.background[0].awaited);
    assert_eq!(s.pending_jobs, vec!["sleep 90".to_string()]);
    assert!(s.alerts.is_empty());

    // Blocked: one alert, not repeated while it stays blocked.
    s.job_output("bxyho0k31", None, vec![step("Dev", "blocked")], Some(8), 9);
    assert_eq!(s.alerts.len(), 1);
    assert_eq!(s.alerts[0].kind, AlertKind::Job);
    assert_eq!(s.alerts[0].text, "sleep 90: website: Dev needs approval");
    let id = s.alerts[0].id;
    assert!(!s.job_output("bxyho0k31", None, vec![step("Dev", "blocked")], Some(8), 10));
    assert_eq!(s.alerts[0].id, id);

    // An item missing from a later tail keeps its state; another item is added.
    let other = Progress {
        item: Some("importer".into()),
        done: Some(1),
        total: Some(3),
        ..Default::default()
    };
    s.job_output("bxyho0k31", None, vec![other], Some(11), 12);
    assert_eq!(s.background[0].progress.len(), 2);
    assert_eq!(
        s.background[0].progress[0].state.as_deref(),
        Some("blocked")
    );

    // A later Stop keeps what was read, and when it was first seen.
    s.apply(&ours("Stop.background-shell"), 20, &th());
    assert_eq!(s.background[0].progress.len(), 2);
    assert_eq!(
        (s.background[0].seen_at, s.background[0].output_at),
        (Some(5), Some(11))
    );
    assert!(s.background[0].awaited);
    assert_eq!(s.alerts.len(), 1);

    // Approved and moving on: the alert goes. Failing doesn't raise one (Claude reports it).
    s.job_output(
        "bxyho0k31",
        None,
        vec![step("Dev", "running")],
        Some(21),
        22,
    );
    assert!(s.alerts.is_empty());
    s.job_output("bxyho0k31", None, vec![step("Dev", "failed")], Some(23), 24);
    assert!(s.alerts.is_empty());

    // Blocked when the job ends (its last line never read): the alert goes with it.
    s.job_output(
        "bxyho0k31",
        None,
        vec![step("Dev", "blocked")],
        Some(25),
        26,
    );
    assert_eq!(s.alerts.len(), 1);
    s.apply(&ours("Stop"), 30, &th());
    assert!(s.background.is_empty());
    assert!(s.alerts.iter().all(|a| a.kind != AlertKind::Job));
}

#[test]
fn background_agent_task_list_is_its_progress() {
    let mut s = Session::new(SID, 0);
    s.apply(&ours("UserPromptSubmit"), 1, &th());
    s.apply(&ours("Stop.background-tasks"), 2, &th());
    let todo = fx_with(
        "PreToolUse",
        serde_json::json!({ "session_id": SID, "agent_id": "a8c13db353cf1070d",
            "tool_name": "TodoWrite", "tool_input": { "todos": [
                { "content": "Read", "status": "completed", "activeForm": "Reading" },
                { "content": "Fix", "status": "in_progress", "activeForm": "Fixing the parser" },
                { "content": "Test", "status": "pending", "activeForm": "Testing" }]}}),
    );
    s.apply(&todo, 3, &th());
    let b = s.background.iter().find(|b| b.agent).unwrap();
    assert_eq!(b.progress.len(), 1);
    assert_eq!(
        (b.progress[0].done, b.progress[0].total),
        (Some(1), Some(3))
    );
    assert_eq!(b.progress[0].detail.as_deref(), Some("Fixing the parser"));
    // The session's own task list is untouched.
    assert!(s.todos.is_none());
}

#[test]
fn headless_sessions_find_their_output_folder_in_the_transcript() {
    // `claude -p` sends no scratchpad_dir: the job has an id but no folder to read.
    let mut st = Store::default();
    st.apply(
        &fx_with(
            "Stop.background-shell",
            serde_json::json!({"session_id": SID, "transcript_path": "/t.jsonl"}),
        ),
        0,
    );
    assert_eq!(st.jobs_without_ids().len(), 1);
    let found = [crate::jobs::Started {
        description: "sleep 90".into(),
        id: "bxyho0k31".into(),
        output: "/tmp/claude-1000/-p/s1/tasks/bxyho0k31.output".into(),
    }];
    assert!(st.set_job_ids(SID, &found));
    assert_eq!(
        st.get(SID).unwrap().tasks_dir.as_deref(),
        Some("/tmp/claude-1000/-p/s1/tasks")
    );
    assert!(st.jobs_without_ids().is_empty());
    assert_eq!(st.job_files().len(), 1);
}
