// Pure view logic, shared by components and unit-tested in view.test.ts.
import type { AwaySpell, Session, UiState } from "./types";

/** Removed by the user (or `/exit`): in its project's "closed" section. */
export const removed = (s: Session) => s.removed === true;

/** Not running, but still in the list (not removed): its process ended (sitting idle with
 * nothing showing it, a tab closed, a crash). To you it's a ready conversation; a click
 * resumes it. */
export const dormant = (s: Session) => s.status === "closed" && !removed(s);

/** In the list, as far as the user is concerned: everything they haven't removed. */
export const isOpen = (s: Session) => !removed(s);

/** Within a project: the sessions in the list (in the order the user dragged them into, the
 * rest by start, so a new one goes to the bottom), then the removed ones, newest first. */
export function arrange(
  sessions: Session[],
  order: string[] = [],
): { visible: Session[]; closed: Session[] } {
  const byStart = (a: Session, b: Session) => a.started_at - b.started_at;
  const open = sessions.filter(isOpen).sort(byStart);
  const closed = sessions.filter(removed).sort((a, b) => b.status_since - a.status_since);
  const rank = (s: Session) => {
    const i = order.indexOf(s.session_id);
    return i < 0 ? Infinity : i;
  };
  // Array.sort is stable: unranked sessions keep the usual order.
  const visible = open.sort((a, b) => rank(a) - rank(b));
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
  // Running ones: what "N open" and a drawer tile's dot are about.
  const open = sessions.filter((s) => s.status !== "closed");
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

/** Time from `from` to `now` that the user was at the computer: the away spells (lunch, a
 * night, the computer off) don't count. */
export function presentTime(from: number, now: number, away: AwaySpell[]): number {
  let gone = 0;
  for (const [start, end] of away) {
    const a = Math.max(start, from);
    const b = Math.min(end ?? now, now);
    if (b > a) gone += b - a;
  }
  return Math.max(0, now - from - gone);
}
