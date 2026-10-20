// Pure view logic, shared by components and unit-tested in view.test.ts.
import type { Session, UiState } from "./types";

/** Within a project: starred first (open or closed), then open, then closed (newest first). */
export function arrange(sessions: Session[]): { visible: Session[]; closed: Session[] } {
  const byStart = (a: Session, b: Session) => a.started_at - b.started_at;
  const starred = sessions.filter((s) => s.starred).sort(byStart);
  const open = sessions.filter((s) => !s.starred && s.status !== "closed").sort(byStart);
  const closed = sessions
    .filter((s) => !s.starred && s.status === "closed")
    .sort((a, b) => b.status_since - a.status_since);
  return { visible: [...starred, ...open], closed };
}

const ATTENTION = new Set(["needs_permission", "needs_input", "error"]);

/** Keys of alerts and permission prompts, for the "just appeared" flash. */
export function freshKeys(s: Session): string[] {
  const keys = s.alerts.map((a) => `${s.session_id}:${a.id}`);
  if (s.status === "needs_permission" && s.pending_permission)
    keys.push(`${s.session_id}:perm:${s.pending_permission.since}`);
  return keys;
}

export interface ProjectStatus {
  open: number;
  /** Dot state: "attention" > "working" > "open" > "" (nothing open). */
  state: "attention" | "working" | "open" | "";
  flashing: boolean;
}

/** Summary for a project header or strip dot. */
export function projectStatus(sessions: Session[], fresh: Set<string>): ProjectStatus {
  const open = sessions.filter((s) => s.status !== "closed");
  const attention = open.some(
    (s) => ATTENTION.has(s.status) || s.alerts.some((a) => a.kind !== "compact"),
  );
  const working = open.some((s) => s.status === "working" || s.status === "background");
  const flashing = sessions.some((s) => freshKeys(s).some((k) => fresh.has(k)));
  return {
    open: open.length,
    state: attention ? "attention" : working ? "working" : open.length ? "open" : "",
    flashing,
  };
}

/** Sessions by tracked project root; the rest (project unknown or untracked) go to `other`. */
export function groupSessions(state: UiState): {
  byRoot: Map<string, Session[]>;
  other: Session[];
} {
  const tracked = new Set(state.projects.map((t) => t.project.root));
  const byRoot = new Map<string, Session[]>();
  const other: Session[] = [];
  for (const s of state.sessions) {
    const root = s.project?.root;
    if (root && tracked.has(root)) byRoot.set(root, [...(byRoot.get(root) ?? []), s]);
    else other.push(s);
  }
  return { byRoot, other };
}

/** Where a project moved from index `from` ends up when dropped before or after the project at
 * index `at` (an index in the list once it has been taken out, as `move_project` expects). */
export function dropIndex(from: number, at: number, place: "before" | "after"): number {
  return (place === "after" ? at + 1 : at) - (from < at ? 1 : 0);
}
