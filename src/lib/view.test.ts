import { describe, expect, it } from "vitest";
import type { Session, UiState } from "./types";
import { arrange, flyoutPosition, freshKeys, groupSessions, projectStatus } from "./view";
import { elapsed } from "./format";

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
    recap: null,
    recap_haiku: null,
    recap_detail: null,
    stop_seq: 0,
    pending_permission: null,
    subagents_running: 0,
    background_running: 0,
    alerts: [],
    closed_reason: null,
    starred: false,
    ...over,
  };
}

describe("arrange", () => {
  it("puts starred first, then open by start, with closed (newest first) separate", () => {
    const a = session({ started_at: 3 });
    const b = session({ started_at: 1 });
    const c = session({ status: "closed", status_since: 5 });
    const d = session({ status: "closed", status_since: 9 });
    const e = session({ status: "closed", starred: true, started_at: 99 });
    const { visible, closed } = arrange([a, b, c, d, e]);
    expect(visible.map((s) => s.session_id)).toEqual([e, b, a].map((s) => s.session_id));
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
      projects: [{ project: a.project!, added_at: 0 }],
      sessions: [a, b, c],
      task_projects: [],
      collapsed: false,
      floating: false,
      left_edge: false,
    };
    const { byRoot, other } = groupSessions(state);
    expect(byRoot.get("/p/a")).toEqual([a]);
    expect(other).toEqual([b, c]);
  });
});

describe("flyoutPosition", () => {
  it("opens below when there is room, above near the bottom", () => {
    expect(flyoutPosition({ top: 100, bottom: 200 }, 1000)).toEqual({ top: 204, maxHeight: 788 });
    expect(flyoutPosition({ top: 800, bottom: 900 }, 1000)).toEqual({
      bottom: 204,
      maxHeight: 788,
    });
  });
});

describe("elapsed", () => {
  it("formats durations", () => {
    expect(elapsed(0, 42_000)).toBe("42s");
    expect(elapsed(0, 5 * 60_000)).toBe("5m");
    expect(elapsed(0, (3 * 60 + 12) * 60_000)).toBe("3h 12m");
    expect(elapsed(0, 3 * 3_600_000)).toBe("3h");
    expect(elapsed(0, 50 * 3_600_000)).toBe("2d");
    expect(elapsed(10, 0)).toBe("0s");
  });
});
