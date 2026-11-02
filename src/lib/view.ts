// Pure view logic, shared by components and unit-tested in view.test.ts.
import type { Background, Progress, Session, Todo, UiState } from "./types";
import { elapsed } from "./format";

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

/** Ready for a prompt, but Claude is waiting on background work it will act on (a background
 * agent, a watcher): it carries on by itself, so it doesn't need the user. Services don't count. */
export function waiting(s: Session): boolean {
  return s.status === "idle" && s.background.some((b) => b.awaited);
}

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
  const working = open.some((s) => s.status === "working" || waiting(s));
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

const HOUR = 3_600_000;

/** A ready pill goes grey once the prompt cache has expired (the next prompt re-sends
 * everything), or was compacted away. With a 5-minute cache or none known: after an hour. */
export function readyStale(s: Session, now: number): boolean {
  if (s.compacted) return true;
  const expires = s.cache && s.cache.ttl >= HOUR ? s.cache.at + s.cache.ttl : s.status_since + HOUR;
  return now >= expires;
}

/** Caches smaller than this aren't worth a line on the card. */
export const CACHE_SHOWN_FROM = 100_000;

/** The prompt cache line for a card, or null: only while the session isn't working (a turn
 * keeps it warm), and only for sizeable conversations. */
export function cacheState(
  s: Session,
  now: number,
): { warm: boolean; left: number; expires: number; tokens: number; ttl: number } | null {
  if (!s.cache || s.compacted || s.status === "working" || s.removed) return null;
  if (s.cache.tokens < CACHE_SHOWN_FROM) return null;
  const expires = s.cache.at + s.cache.ttl;
  return {
    warm: now < expires,
    left: expires - now,
    expires,
    tokens: s.cache.tokens,
    ttl: s.cache.ttl,
  };
}

/** 908_123 → "908k", 1_250_000 → "1.3M". */
export function tokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

/** How long a job has run and, once its output has been still for a minute, for how long:
 * "12m · quiet 12m". */
export function jobTiming(b: Background, now: number): string | null {
  if (b.seen_at === null) return null;
  const quiet = b.output_at !== null && now - b.output_at >= 60_000;
  return quiet
    ? `${elapsed(b.seen_at, now)} · quiet ${elapsed(b.output_at!, now)}`
    : elapsed(b.seen_at, now);
}

const STEP_MARK: Record<string, string> = {
  running: "●",
  waiting: "◌",
  blocked: "⏸",
  failed: "✗",
  done: "✓",
};

/** A progress item's line: its stages ("Build ✓ ─ Dev ● ─ TST ○"), else a bar for a count
 * ("▰▰▰▱▱▱▱▱ 14/40"), else its state; then the detail. */
export function progressLine(p: Progress): string {
  const parts: string[] = [];
  if (p.steps.length > 0) {
    const at = p.at === null ? -1 : p.steps.indexOf(p.at);
    const finished = p.state === "done";
    parts.push(
      p.steps
        .map((name, i) => {
          const mark =
            finished || i < at ? "✓" : i === at ? (STEP_MARK[p.state ?? "running"] ?? "●") : "○";
          return `${name} ${mark}`;
        })
        .join(" ─ "),
    );
  } else if (p.total !== null && p.total > 0 && p.grew) {
    // A total that grows (later stages listed once they start) would make the bar look nearly
    // full throughout: just the count, open-ended.
    parts.push(`${p.done ?? 0}/${p.total}+`);
  } else if (p.total !== null && p.total > 0) {
    const done = Math.min(p.done ?? 0, p.total);
    const filled = Math.round((done / p.total) * 8);
    parts.push(`${"▰".repeat(filled)}${"▱".repeat(8 - filled)} ${done}/${p.total}`);
  } else if (p.state) {
    parts.push(p.state);
  }
  if (p.detail) parts.push(p.detail);
  return parts.join(" · ");
}

/** Claude's last answer as the card shows it: no blank lines (they only take room), and without
 * the question it ends on, which is pinned below it instead (so it's never clipped). */
export function answerBody(answer: string, question: string | null): string {
  const lines = answer.split("\n").filter((l) => l.trim());
  const q = question?.trim();
  const last = lines[lines.length - 1]?.trimEnd();
  if (q && last?.endsWith(q)) {
    const rest = last
      .slice(0, -q.length)
      .replace(/^\s*•\s*$/, "")
      .trimEnd();
    if (rest) lines[lines.length - 1] = rest;
    else lines.pop();
  }
  return lines.join("\n");
}

/** A project's to-do items still to do, in the user's order. */
export function openTodos(todos: Todo[]): Todo[] {
  return todos.filter((t) => t.state !== "done");
}

/** A to-do's text split into plain parts and links (http/https), for rendering; a link shows
 * as its host and last path part. A trailing full stop or bracket stays outside the link. */
export function todoParts(text: string): { text: string; url?: string }[] {
  const parts: { text: string; url?: string }[] = [];
  const re = /https?:\/\/[^\s<>"]+[^\s<>".,;:!?)\]']/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    if (m.index > last) parts.push({ text: text.slice(last, m.index) });
    parts.push({ text: shortUrl(m[0]), url: m[0] });
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push({ text: text.slice(last) });
  return parts;
}

/** "https://dev.example.com/org/proj/_wiki/wikis/x/1894" → "dev.example.com/…/1894". */
export function shortUrl(url: string): string {
  try {
    const u = new URL(url);
    const segs = u.pathname.split("/").filter(Boolean);
    if (segs.length === 0) return u.host;
    const last = decodeURIComponent(segs[segs.length - 1]);
    return segs.length === 1 ? `${u.host}/${last}` : `${u.host}/…/${last}`;
  } catch {
    return url;
  }
}
