// Pure view logic, shared by components and unit-tested in view.test.ts.
import type { Session, UiState } from "./types";

/** Ended by the sidebar for sitting idle with nothing showing it (tmux sessions): only the
 * process went, to save resources. To you it's still a ready conversation; a click resumes it. */
export const dormant = (s: Session) => s.status === "closed" && s.closed_reason === "idle";

/** Open as far as the user is concerned: running, or dormant. */
export const isOpen = (s: Session) => s.status !== "closed" || dormant(s);

/** Within a project: starred first (open or closed), then open, then closed (newest first).
 * `order` (session ids the user dragged into order) comes first among the visible ones; the
 * rest follow in the usual order, so a new session goes to the bottom. */
export function arrange(
  sessions: Session[],
  order: string[] = [],
): { visible: Session[]; closed: Session[] } {
  const byStart = (a: Session, b: Session) => a.started_at - b.started_at;
  const starred = sessions.filter((s) => s.starred).sort(byStart);
  const open = sessions.filter((s) => !s.starred && isOpen(s)).sort(byStart);
  const closed = sessions
    .filter((s) => !s.starred && !isOpen(s))
    .sort((a, b) => b.status_since - a.status_since);
  const rank = (s: Session) => {
    const i = order.indexOf(s.session_id);
    return i < 0 ? Infinity : i;
  };
  // Array.sort is stable: unranked sessions keep the usual order.
  const visible = [...starred, ...open].sort((a, b) => rank(a) - rank(b));
  return { visible, closed };
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
  const open = sessions.filter(isOpen);
  const attention = open.some(
    (s) => ATTENTION.has(s.status) || s.alerts.some((a) => a.kind !== "compact"),
  );
  const working = open.some((s) => s.status === "working");
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

/** Ids of `ids` with `moved` taken out and put just before or after `target`. */
export function reorder(
  ids: string[],
  moved: string,
  target: string,
  place: "before" | "after",
): string[] {
  const rest = ids.filter((id) => id !== moved);
  const at = rest.indexOf(target);
  if (at < 0) return ids;
  rest.splice(place === "after" ? at + 1 : at, 0, moved);
  return rest;
}
