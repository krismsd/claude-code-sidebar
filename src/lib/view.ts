// Pure view logic, shared by components and unit-tested in view.test.ts.
import type { Alert, Background, Progress, Session, UiState } from "./types";
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

/** Ready (or dormant) with a finished turn the user hasn't looked at (SPEC §3.17). Not while
 * waiting on background work: Claude isn't done yet (its report-back turn sets it again). */
export function unread(s: Session): boolean {
  return s.unread && (s.status === "idle" || dormant(s)) && !waiting(s);
}

/** Keys of alerts and permission prompts, for the "just appeared" flash. */
export function freshKeys(s: Session): string[] {
  const keys = s.alerts.map((a) => `${s.session_id}:${a.id}`);
  if (s.status === "needs_permission" && s.pending_permission)
    keys.push(`${s.session_id}:perm:${s.pending_permission.since}`);
  return keys;
}

/** Alerts that only inform: compaction, a background job reporting back, a PR merged (the
 * text is `store.rs`'s "PR merged: …"; failing checks and changes requested do need you). */
export function informational(a: Alert): boolean {
  return (
    a.kind === "compact" ||
    a.kind === "finished" ||
    (a.kind === "pr" && a.text.startsWith("PR merged"))
  );
}

/** Needs the user: a permission prompt, a question, an error, or an alert that isn't just news. */
export function needsYou(s: Session): boolean {
  return ATTENTION.has(s.status) || s.alerts.some((a) => !informational(a));
}

/** What a collapsed-strip marker shows (SPEC §3.17). A dot only ever means unread. */
export type Marker = "error" | "attention" | "working" | "waiting" | "unread" | "read";

export function marker(s: Session): Marker {
  if (s.status === "error") return "error";
  if (needsYou(s)) return "attention";
  if (s.status === "working") return "working";
  if (waiting(s)) return "waiting";
  return unread(s) ? "unread" : "read";
}

export interface ProjectStatus {
  open: number;
  /** Drawer tile mark: "attention" (!) > "unread" (green dot) > "" (nothing). */
  state: "attention" | "unread" | "";
  flashing: boolean;
}

/** Summary for a project header or drawer tile. */
export function projectStatus(sessions: Session[], fresh: Set<string>): ProjectStatus {
  // Running ones: what "N open" is about.
  const open = sessions.filter((s) => s.status !== "closed");
  const listed = sessions.filter(isOpen);
  const flashing = sessions.some((s) => freshKeys(s).some((k) => fresh.has(k)));
  return {
    open: open.length,
    state: open.some(needsYou) ? "attention" : listed.some(unread) ? "unread" : "",
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
/** "12m left", or "over by 3m" once the expected end has passed. */
export function countdown(until: number, now: number): string {
  return until > now ? `${elapsed(now, until)} left` : `over by ${elapsed(until, now)}`;
}

const bar = (frac: number) => {
  const filled = Math.round(Math.min(1, Math.max(0, frac)) * 8);
  return `${"▰".repeat(filled)}${"▱".repeat(8 - filled)}`;
};

/** Stages that fit on a card line in full; longer lists are shortened around the current one. */
const STEPS_IN_FULL = 4;

/** A step list as a line: "Build ✓ ─ Dev ● ─ TST ○". A long one (a pipeline of 11 stages)
 * would be cut off at the card's edge, hiding the current stage, so it keeps the current stage
 * and the next one and counts the rest: "10 ✓ ─ Release ●", "4 ✓ ─ Migrate ● ─ Infra ○ +5",
 * "all 11 ✓". */
export function stepsLine(p: Progress): string {
  const at = p.at === null ? -1 : p.steps.indexOf(p.at);
  const finished = p.state === "done";
  const mark = (i: number) =>
    finished || i < at ? "✓" : i === at ? (STEP_MARK[p.state ?? "running"] ?? "●") : "○";
  const n = p.steps.length;
  if (n <= STEPS_IN_FULL) return p.steps.map((name, i) => `${name} ${mark(i)}`).join(" ─ ");
  if (finished) return `all ${n} ✓`;
  const cur = Math.max(at, 0);
  const shown: string[] = [];
  if (cur > 0) shown.push(`${cur} ✓`);
  shown.push(`${p.steps[cur]} ${mark(cur)}`);
  if (cur + 1 < n) shown.push(`${p.steps[cur + 1]} ○`);
  const rest = n - cur - 2;
  return shown.join(" ─ ") + (rest > 0 ? ` +${rest}` : "");
}

/** A job's name on its card line. Claude names most background commands; one started without a
 * description gets its command as the name, which is unreadable (a whole watcher loop, or
 * "bash /tmp/…/watch-pipeline.sh 1234…"). Then: what its progress links to (a CI
 * run), else the command shortened to the script it runs. */
export function jobTitle(b: Background): string {
  if (b.agent) return b.description || "Agent";
  const described = b.description.trim();
  if (described && described !== b.command.trim()) return described;
  const url = b.progress.find((p) => p.url)?.url;
  if (url) {
    const gh = /github\.com\/[^/]+\/([^/]+)\/actions\/runs\/(\d+)/.exec(url);
    if (gh) return `GitHub run ${gh[2]} · ${gh[1]}`;
    const gl = /\/([^/]+)\/-\/pipelines\/(\d+)/.exec(url);
    if (gl) return `GitLab pipeline ${gl[2]} · ${gl[1]}`;
    try {
      return new URL(url).host;
    } catch {
      /* not a URL: fall through */
    }
  }
  const words = b.command.trim().split(/\s+/);
  if (words.length >= 2 && /^(bash|sh|zsh|python3?|node|deno|bun|npx|tsx)$/.test(words[0]))
    words.shift();
  if (words[0]?.includes("/")) words[0] = words[0].split("/").pop()!;
  return words.join(" ") || "Command";
}

/** One progress item as a line. With an end time (`until`) and `now`, a countdown: a time bar
 * from when `until` was given (else `seen`, when the job was first seen) when there's nothing
 * else to show, otherwise appended. */
export function progressLine(p: Progress, now?: number, seen?: number | null): string {
  const parts: string[] = [];
  const timed = p.until != null && now !== undefined && p.state !== "done";
  const start = p.since ?? seen;
  if (p.steps.length > 0) {
    parts.push(stepsLine(p));
  } else if (p.total !== null && p.total > 0 && p.grew) {
    // A total that grows (later stages listed once they start) would make the bar look nearly
    // full throughout: just the count, open-ended.
    parts.push(`${p.done ?? 0}/${p.total}+`);
  } else if (p.total !== null && p.total > 0) {
    const done = Math.min(p.done ?? 0, p.total);
    parts.push(`${bar(done / p.total)} ${done}/${p.total}`);
  } else if (timed) {
    const left = countdown(p.until!, now!);
    parts.push(
      start != null && p.until! > start
        ? `${bar((now! - start) / (p.until! - start))} ${left}`
        : left,
    );
  } else if (p.state) {
    parts.push(p.state);
  }
  if (timed && (p.steps.length > 0 || (p.total !== null && p.total > 0)))
    parts.push(countdown(p.until!, now!));
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

/** A project's emoji as typed: only the last character the user entered (an emoji built from
 * several, such as 👨‍💻 or a flag, counts as one), so a second pick or a typed word can't pile
 * up in a mark drawn in a small tile. */
export function oneEmoji(text: string): string {
  const parts = [
    ...new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(text.trim()),
  ];
  return parts.length ? parts[parts.length - 1].segment : "";
}
