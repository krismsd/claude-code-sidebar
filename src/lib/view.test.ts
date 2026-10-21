import { describe, expect, it } from "vitest";
import type { Session, UiState } from "./types";
import {
  arrange,
  dropIndex,
  freshKeys,
  groupSessions,
  presentTime,
  projectStatus,
  reorder,
} from "./view";
import { elapsed, sessionName, usageLevel, usageNow } from "./format";

let n = 0;
function session(over: Partial<Session> = {}): Session {
  n += 1;
  return {
    session_id: `s${n}`,
    tag: "t",
    pid: 1,
    wt_session: null,
    distro: null,
    cwd: "/p/a",
    shell_cwd: null,
    project: {
      root: "/p/a",
      name: "a",
      color: "#000",
      emoji: null,
      label: null,
      worktree: null,
      branch: "main",
    },
    transcript_path: null,
    permission_mode: null,
    model: null,
    status: "idle",
    status_since: 0,
    started_at: n,
    last_event_at: 0,
    current_tool: null,
    last_prompt: null,
    first_prompt: null,
    title: null,
    user_title: null,
    user_title_synced: false,
    start_source: null,
    recap: null,
    recap_haiku: null,
    recap_detail: null,
    last_answer: null,
    question: null,
    stop_seq: 0,
    pending_permission: null,
    subagents_running: 0,
    background_running: 0,
    background: [],
    tmux: null,
    attached: null,
    alerts: [],
    closed_reason: null,
    removed: false,
    ...over,
  };
}

describe("arrange", () => {
  it("keeps every session not removed in the list, by start; removed ones newest first", () => {
    const a = session({ started_at: 3 });
    const b = session({ started_at: 1 });
    const c = session({ status: "closed", removed: true, status_since: 5 });
    const d = session({ status: "closed", removed: true, status_since: 9 });
    const e = session({ status: "closed", started_at: 99 });
    const { visible, closed } = arrange([a, b, c, d, e]);
    expect(visible.map((s) => s.session_id)).toEqual([b, a, e].map((s) => s.session_id));
    expect(closed.map((s) => s.session_id)).toEqual([d, c].map((s) => s.session_id));
  });
});

describe("projectStatus", () => {
  it("ranks attention over working over open", () => {
    expect(projectStatus([session({ status: "closed" })], new Set()).state).toBe("");
    expect(projectStatus([session()], new Set()).state).toBe("open");
    expect(projectStatus([session(), session({ status: "working" })], new Set()).state).toBe(
      "working",
    );
    const waiting = session({ status: "needs_input" });
    expect(projectStatus([session({ status: "working" }), waiting], new Set()).state).toBe(
      "attention",
    );
  });

  it("treats non-compact alerts as attention, and flashes on fresh keys", () => {
    const compact = session({ alerts: [{ id: 1, kind: "compact", text: "", at: 0 }] });
    expect(projectStatus([compact], new Set()).state).toBe("open");
    const denied = session({ alerts: [{ id: 2, kind: "denied", text: "", at: 0 }] });
    const st = projectStatus([denied], new Set(freshKeys(denied)));
    expect(st).toEqual({ open: 1, state: "attention", flashing: true });
  });

  it("includes permission prompts in fresh keys", () => {
    const s = session({
      status: "needs_permission",
      pending_permission: { tool: "Bash", summary: "x", since: 42, answerable: true },
    });
    expect(freshKeys(s)).toEqual([`${s.session_id}:perm:42`]);
  });
});

describe("groupSessions", () => {
  it("groups by tracked root and leaves the rest in other", () => {
    const a = session();
    const b = session({ project: null, cwd: "/x" });
    const c = session({ project: { ...a.project!, root: "/p/untracked" } });
    const state: UiState = {
      projects: [{ project: a.project!, added_at: 0, shelved: false, session_order: [] }],
      sessions: [a, b, c],
      task_projects: [],
      collapsed: false,
      floating: false,
      left_edge: false,
      monitor: 0,
      usage: null,
      away: [],
    };
    const { byRoot, other } = groupSessions(state);
    expect(byRoot.get("/p/a")).toEqual([a]);
    expect(other).toEqual([b, c]);
  });
});

describe("elapsed", () => {
  it("formats durations", () => {
    expect(elapsed(0, 42_000)).toBe("42s");
    expect(elapsed(0, 5 * 60_000)).toBe("5m");
    expect(elapsed(0, (3 * 60 + 12) * 60_000)).toBe("3h");
    expect(elapsed(0, (5 * 60 + 36) * 60_000)).toBe("5h");
    expect(elapsed(0, 3 * 3_600_000)).toBe("3h");
    expect(elapsed(0, 50 * 3_600_000)).toBe("2d");
    expect(elapsed(10, 0)).toBe("0s");
  });
});

describe("sessionName", () => {
  it("prefers the title, then the first prompt", () => {
    expect(sessionName(session())).toBe("New session");
    expect(sessionName(session({ first_prompt: "Fix the login bug" }))).toBe("Fix the login bug");
    expect(sessionName(session({ first_prompt: "Fix it", title: "Login fixes" }))).toBe(
      "Login fixes",
    );
    expect(sessionName(session({ title: "Login fixes", user_title: "Mine" }))).toBe("Mine");
  });
});

describe("usageLevel", () => {
  it("goes amber at 70% and red at 90%", () => {
    expect([0, 69.9, 70, 89.9, 90, 100].map(usageLevel)).toEqual([
      "ok",
      "ok",
      "high",
      "high",
      "critical",
      "critical",
    ]);
  });
});

describe("usageNow", () => {
  it("drops the old percentage once the window has reset", () => {
    const w = { percent: 8, resets_at: 1000 };
    expect(usageNow(w, 999)).toEqual({ percent: 8, reset: false });
    expect(usageNow(w, 1000)).toEqual({ percent: 0, reset: true });
    expect(usageNow({ percent: 5, resets_at: null }, 1e15)).toEqual({ percent: 5, reset: false });
    expect(usageNow(null, 0)).toBeNull();
  });
});

describe("dropIndex", () => {
  // [a, b, c, d]
  it("moves down: dropping a after c puts it third", () => {
    expect(dropIndex(0, 2, "after")).toBe(2); // [b, c, a, d]
    expect(dropIndex(0, 2, "before")).toBe(1); // [b, a, c, d]
  });
  it("moves up", () => {
    expect(dropIndex(3, 1, "before")).toBe(1); // [a, d, b, c]
    expect(dropIndex(3, 1, "after")).toBe(2); // [a, b, d, c]
  });
  it("next to itself is no move", () => {
    expect(dropIndex(1, 2, "before")).toBe(1);
    expect(dropIndex(2, 1, "after")).toBe(2);
  });
});

describe("session order", () => {
  it("dragged order first, the rest after in the usual order", () => {
    const a = session({ session_id: "a", started_at: 1 });
    const b = session({ session_id: "b", started_at: 2 });
    const c = session({ session_id: "c", started_at: 3 });
    const ids = (ss: Session[]) => ss.map((s) => s.session_id);
    expect(ids(arrange([a, b, c]).visible)).toEqual(["a", "b", "c"]);
    expect(ids(arrange([a, b, c], ["c", "a"]).visible)).toEqual(["c", "a", "b"]);
  });
  it("reorder puts the moved id next to the target", () => {
    expect(reorder(["a", "b", "c"], "a", "c", "after")).toEqual(["b", "c", "a"]);
    expect(reorder(["a", "b", "c"], "c", "a", "before")).toEqual(["c", "a", "b"]);
    expect(reorder(["a", "b", "c"], "b", "a", "after")).toEqual(["a", "b", "c"]);
  });
});

describe("dormant sessions", () => {
  it("process ended but not removed: shown in the list, not folded away as closed", () => {
    const a = session({ status: "closed", closed_reason: "idle" });
    const b = session({ status: "closed", closed_reason: "other", removed: true });
    const { visible, closed } = arrange([a, b]);
    expect(visible).toEqual([a]);
    expect(closed).toEqual([b]);
  });
});

describe("presentTime", () => {
  const H = 3_600_000;
  it("skips away spells, including one still going", () => {
    expect(presentTime(0, 3 * H, [])).toBe(3 * H);
    // Ready at 0, lunch from 1h to 2h: 2h at the computer by 3h.
    expect(presentTime(0, 3 * H, [[H, 2 * H]])).toBe(2 * H);
    // Away since before it became ready, still away.
    expect(presentTime(H, 10 * H, [[0, null]])).toBe(0);
    // Only the part after it became ready counts.
    expect(presentTime(2 * H, 4 * H, [[H, 3 * H]])).toBe(H);
  });
});
