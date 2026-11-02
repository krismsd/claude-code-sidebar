import { useState, useRef, type CSSProperties } from "react";
import type { Alert, Background, PullRequest, Session } from "../lib/types";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  answerBody,
  cacheState,
  dormant,
  jobTiming,
  progressLine,
  readyStale,
  waiting,
  tokens,
} from "../lib/view";
import { ALERT_EMOJI, STATUS_LABEL, closedLabel, elapsed, sessionName } from "../lib/format";

interface Props {
  session: Session;
  now: number;
  /** Alert/permission keys that just appeared (see App useFresh). */
  fresh: Set<string>;
  onActivate: (s: Session) => void;
  onDismissAlert: (s: Session, alertId: number) => void;
  onAnswer: (s: Session, allow: boolean) => void;
  /** Hover in/out, with the card element so the expanded copy can be drawn over it. Called
   * again on every movement over the body, so the wait for the copy counts from a rest. */
  onHover: (s: Session, el: HTMLElement | null) => void;
  /** Right-click: the session menu at the cursor. */
  onMenu: (s: Session, x: number, y: number) => void;
  /** Minimise the card to its title bar, or expand it. */
  onMinimize: (s: Session, minimized: boolean) => void;
  /** The hover copy (and the strip's peek): what the card cuts off, shown in full. */
  expanded?: boolean;
  /** Selected with the keyboard (arrow keys). */
  focused?: boolean;
}

const stop =
  (f: () => void) =>
  (e: { stopPropagation: () => void }): void => {
    e.stopPropagation();
    f();
  };

const clock = (ms: number) =>
  new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

function cacheDetail(c: NonNullable<ReturnType<typeof cacheState>>): string {
  const life = c.ttl >= 3_600_000 ? "1 h" : `${Math.round(c.ttl / 60_000)} min`;
  return `Prompt cache: ${tokens(c.tokens)} tokens, ${life} lifetime, ${c.warm ? "expires" : "expired"} ${clock(c.expires)}`;
}

/** The newest steps that fit on one card line (about 40 characters), oldest first. */
function recentSteps(trail: { label: string; count: number }[]) {
  const out: typeof trail = [];
  let width = 0;
  for (const t of [...trail].reverse()) {
    width += [...t.label].length + (t.count > 1 ? 3 : 0) + 3;
    if (out.length > 0 && width > 40) break;
    out.unshift(t);
  }
  return out;
}

/** Whether the hover copy would show more than the card: a clamped or cut-off line, or steps
 * and progress items left out (marked `data-more`). Only then does hovering open it. */
function hasMore(card: HTMLElement): boolean {
  if (card.querySelector("[data-more]")) return true;
  const cut = card.querySelectorAll<HTMLElement>(
    ".card-recap, .card-job-name, .card-job-output, .card-job-steps",
  );
  return [...cut].some(
    (el) => el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1,
  );
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Session card. Compact in the list (its height never changes); the hover overlay draws the
 * same card `expanded` on top of it, so the top part lines up exactly. */
export function SessionCard({
  session: s,
  now,
  fresh,
  onActivate,
  onDismissAlert,
  onAnswer,
  onHover,
  onMenu,
  onMinimize,
  expanded = false,
  focused = false,
}: Props) {
  const isFresh = (alertId: number) => fresh.has(`${s.session_id}:${alertId}`);
  const flashing =
    s.alerts.some((a) => isFresh(a.id)) ||
    (s.pending_permission !== null &&
      fresh.has(`${s.session_id}:perm:${s.pending_permission.since}`));
  // ⏳ waiting and ✅ finished only flash the card and play their sound: the status pill
  // already says it.
  const shown = s.alerts.filter((a) => a.kind !== "waiting" && a.kind !== "finished");
  // Jobs Claude will act on when they finish (watchers, background agents) get their own
  // highlighted line; other background commands (dev servers) and agents working inside the
  // current turn are just counted.
  const jobs = s.background.filter((b) => b.awaited);
  const services = s.background.filter((b) => !b.awaited && !b.agent);
  const agents = Math.max(0, s.subagents_running - jobs.filter((b) => b.agent).length);
  const jobName = (b: (typeof jobs)[number]) => b.description || (b.agent ? "Agent" : "Command");
  // Newest first: the alert band shows the first, "+N" for the rest.
  const alerts = [...shown].sort((a, b) => b.at - a.at);
  // A dormant session (ended for sitting idle) looks and acts like a ready one.
  const asleep = dormant(s);
  const closed = s.status === "closed" && !asleep;
  const status = asleep ? "idle" : s.status;
  // Ready, with Claude waiting on background work it will come back to (SPEC §3.17).
  const busy = waiting(s);
  // Minimised: just the title bar, unless it needs the user (permission, a question, an
  // error), when it opens until that's dealt with. The hover copy always shows everything.
  const needsYou = ["needs_permission", "needs_input", "error"].includes(s.status);
  const folded = s.minimized && !expanded && !needsYou;
  const overBody = useRef(false);
  const activity =
    s.status === "needs_permission" && s.pending_permission
      ? `Wants to: ${s.pending_permission.summary}`
      : s.status === "working" || s.status === "needs_input"
        ? (s.current_tool ?? (s.status === "working" ? "Thinking…" : null))
        : null;
  // Open PRs (or not looked up yet); merged and closed ones are done with.
  const prs = s.prs.filter((p) => p.state !== "merged" && p.state !== "closed");
  const cache = s.status === "closed" && !asleep ? null : cacheState(s, now);
  // The status band (last footer band): counts and markers that are only information.
  const statusBand =
    services.length > 0 || agents > 0 || (cache !== null && !cache.warm) || s.compacted !== null;
  // A turn in progress: the body shows the turn (prompt, progress, steps), not the last answer.
  const turn = ["working", "needs_permission", "needs_input"].includes(s.status);
  const label = [s.project?.worktree && `⑂ ${s.project.worktree}`, s.project?.branch]
    .filter(Boolean)
    .join(" · ");

  return (
    <article
      className={`card status-${status}${flashing ? " flash" : ""}${closed ? " is-closed" : ""}${folded ? " minimized" : ""}${focused ? " kb-focus" : ""}`}
      data-session={expanded ? undefined : s.session_id}
      style={{ "--project": s.project?.color ?? "#888" } as CSSProperties}
      onClick={() => onActivate(s)}
      // The title bar and PR links are controls (the fold toggle, opening a PR), so only the
      // rest of the body opens the hover copy, which would cover them; a minimised card, all
      // title bar, never does. Nor does a card that already shows everything.
      onMouseOver={(e) => {
        const body =
          !folded &&
          !(e.target as Element).closest(".card-head, .card-footer") &&
          hasMore(e.currentTarget);
        if (body !== overBody.current) {
          overBody.current = body;
          onHover(s, body ? e.currentTarget : null);
        }
      }}
      onMouseMove={(e) => {
        if (overBody.current) onHover(s, e.currentTarget);
      }}
      onMouseLeave={() => {
        overBody.current = false;
        onHover(s, null);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(s, e.clientX, e.clientY);
      }}
    >
      <header className="card-head">
        {/* Fold toggle, in the header's left padding (the title doesn't move): always shown on
            a minimised card, on hover otherwise. Not on the hover copy. */}
        {!expanded && (
          <button
            className={`fold${folded ? " folded" : ""}`}
            title={folded ? "Expand" : "Minimise"}
            onClick={stop(() => onMinimize(s, !s.minimized))}
          >
            {folded ? "▸" : "▾"}
          </button>
        )}
        {/* The prompt in the title's tooltip: on the body it would cover the hover copy. */}
        <span
          className="card-title"
          title={!turn && s.last_prompt ? `You: ${s.last_prompt}` : undefined}
        >
          <span className="card-name">{sessionName(s)}</span>
          {label && <span className="card-branch"> · {label}</span>}
        </span>
        <span
          className={`chip card-status-block chip-${busy ? "waiting" : status}${status === "idle" && !busy && readyStale(s, now) ? " stale" : ""}`}
          title={
            [
              busy &&
                `Ready for a prompt · waiting on ${jobs.length} ${jobs.every((b) => b.agent) ? "agent" : "job"}${jobs.length === 1 ? "" : "s"}`,
              cache && cacheDetail(cache),
              [s.permission_mode, s.model, s.tag && `#${s.tag}`].filter(Boolean).join(" · "),
            ]
              .filter(Boolean)
              .join("\n") || undefined
          }
        >
          {closed ? closedLabel(s) : busy ? "waiting" : STATUS_LABEL[status]} ·{" "}
          {elapsed(s.status_since, now)}
        </span>
      </header>
      {!folded && (
        <>
          {s.status !== "closed" && s.tag === null && (
            <div className="card-warning" title="Run /exit in that tab, then: claudes --continue">
              ⚠ Started with plain <code>claude</code>: can&apos;t focus or end it. Restart with{" "}
              <code>claudes --continue</code>
            </div>
          )}
          {/* A turn in progress: what you asked, how far along it is, and its recent steps.
          The last answer is old news by now. */}
          {turn && (
            <div className="card-turn-prompt" title={s.turn_note ?? s.last_prompt ?? undefined}>
              ▸ {s.turn_note ?? (s.last_prompt ? `You: ${s.last_prompt}` : "Working")}
            </div>
          )}
          {turn && s.todos && (
            <div className="card-todos" title={`${s.todos.done} of ${s.todos.total} tasks done`}>
              <span className="todo-bar">
                {Array.from({ length: s.todos.total }, (_, i) => (
                  <i key={i} className={i < s.todos!.done ? "done" : ""} />
                ))}
              </span>
              <span className="todo-text">
                {s.todos.done}/{s.todos.total}
                {s.todos.current && ` · ${s.todos.current}`}
              </span>
            </div>
          )}
          {turn && s.trail.length > 0 && (
            <div
              className={`card-trail${expanded ? " full" : ""}`}
              data-more={!expanded && recentSteps(s.trail).length < s.trail.length ? "" : undefined}
            >
              {(expanded ? s.trail : recentSteps(s.trail)).map((t, i) => (
                <span key={i}>
                  {t.label}
                  {t.count > 1 && <span className="muted"> ×{t.count}</span>}
                </span>
              ))}
            </div>
          )}
          {activity && <div className="card-activity">{activity}</div>}
          {s.status === "needs_permission" && s.pending_permission?.answerable && (
            <div className="answer">
              <button className="allow" onClick={stop(() => onAnswer(s, true))}>
                Allow
              </button>
              <button className="deny" onClick={stop(() => onAnswer(s, false))}>
                Deny
              </button>
            </div>
          )}
          {/* Claude's last answer as written: the start on the card, more of it on hover. The
          question it ended on, if any, is pinned below so it's never clipped. */}
          {(s.last_answer ?? s.recap) && !turn && (
            <div className={`card-recap${expanded ? " full" : ""}`}>
              {answerBody(s.last_answer ?? s.recap ?? "", s.question)}
            </div>
          )}
          {s.question && !turn && <div className="card-question">{s.question}</div>}
        </>
      )}
      {/* Footer bands, kept when the card is minimised: jobs Claude will act on when they
      finish (watchers, background agents), then PRs the session opened. */}
      {jobs.length + prs.length + alerts.length + (statusBand ? 1 : 0) > 0 && (
        <footer className="card-footer">
          {jobs.length > 0 && (
            <div className="card-jobs" title={jobs.map(jobName).join("\n")}>
              {jobs.map((b, i) => (
                <JobLine
                  key={b.id || i}
                  job={b}
                  name={`${b.agent ? "🤖" : "⏱"} ${jobName(b)}`}
                  now={now}
                  expanded={expanded}
                  brief={folded}
                />
              ))}
            </div>
          )}
          {prs.length > 0 && (
            <div className="card-prs">
              {prs.map((p) => (
                <PrLine
                  key={`${p.host}${p.number}`}
                  pr={p}
                  repo={repoLabel(p, s.project?.repos)}
                  expanded={expanded}
                />
              ))}
            </div>
          )}
          {alerts.length > 0 && (
            <AlertBand
              alerts={alerts}
              fresh={alerts.some((a) => isFresh(a.id))}
              onDismiss={(id) => onDismissAlert(s, id)}
            />
          )}
          {statusBand && (
            <div className="card-status">
              {services.length > 0 && (
                <span
                  title={[
                    `${plural(services.length, "background task")}:`,
                    ...services.map((b) =>
                      b.last_line ? `${jobName(b)}\n   ${b.last_line}` : jobName(b),
                    ),
                  ].join("\n")}
                >
                  ⧗ {services.length}
                </span>
              )}
              {agents > 0 && <span title={`${plural(agents, "agent")} running`}>🤖 {agents}</span>}
              {cache && !cache.warm && (
                <span
                  className="cold"
                  title={`Next prompt re-sends ${tokens(cache.tokens)}\n${cacheDetail(cache)}`}
                >
                  ❄ {tokens(cache.tokens)}
                </span>
              )}
              {s.compacted && (
                <span
                  title={`${s.compacted.idle ? "Compacted while idle" : "Compacted"} at ${clock(s.compacted.at)}`}
                >
                  📦 {tokens(s.compacted.pre_tokens)}→{tokens(s.compacted.post_tokens)}
                </span>
              )}
            </div>
          )}
        </footer>
      )}
    </article>
  );
}

const CHECKS = { passing: "✓", failing: "✗", running: "⋯" } as const;
const CHECKS_WORDS = {
  passing: "checks passing",
  failing: "checks failing",
  running: "checks running",
} as const;
const REVIEW = { approved: "approved", changes_requested: "changes", required: "review" } as const;
const REVIEW_WORDS = {
  approved: "approved",
  changes_requested: "changes requested",
  required: "review needed",
} as const;

/** The folder in the project that holds a PR's repo, so the user sees the name they know.
 * None when the repo is the project itself; the repo name when it isn't cloned there. */
function repoLabel(pr: PullRequest, repos: Record<string, string> | undefined): string | null {
  if (!pr.repo) return null;
  const folder = repos?.[pr.repo];
  if (folder === "") return null;
  return folder ?? pr.repo;
}

/** An open PR the session opened: repo folder, number, title, and how it stands (checks and
 * review, right-aligned). Click to open it. */
function PrLine({
  pr,
  repo,
  expanded,
}: {
  pr: PullRequest;
  repo: string | null;
  expanded: boolean;
}) {
  const status = [
    pr.state === "draft" && "draft",
    pr.checks && CHECKS[pr.checks],
    pr.review && REVIEW[pr.review],
  ]
    .filter(Boolean)
    .join(" ");
  const words = [
    pr.state === "draft" && "draft",
    pr.checks && CHECKS_WORDS[pr.checks],
    pr.review && REVIEW_WORDS[pr.review],
  ]
    .filter(Boolean)
    .join(", ");
  const bad = pr.checks === "failing" || pr.review === "changes_requested";
  const good = !bad && pr.review === "approved" && pr.checks !== "running";
  const tip = [
    `${pr.repo ?? ""} #${pr.number}${pr.title ? ` ${pr.title}` : ""}`.trim(),
    words,
    pr.url,
  ]
    .filter(Boolean)
    .join("\n");
  return (
    <div
      className={`card-pr${expanded ? " full" : ""}${bad ? " bad" : good ? " good" : ""}`}
      title={tip}
      onClick={(e) => {
        e.stopPropagation();
        if (pr.url) void openUrl(pr.url);
      }}
    >
      <span className="card-pr-name">
        🔀 {repo && <span className="card-pr-repo">{repo}</span>} #{pr.number}{" "}
        {pr.title ?? <span className="muted">…</span>}
      </span>
      {status && <span className="card-pr-status">{status}</span>}
    </div>
  );
}

/** The alert band: the newest alert with ✕ (the next one moves up when it's dismissed) and
 * "+N" for the others, which expands the band into a list, each with ✕, and "Dismiss all". */
function AlertBand({
  alerts,
  fresh,
  onDismiss,
}: {
  alerts: Alert[];
  fresh: boolean;
  onDismiss: (id: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const list = open && alerts.length > 1;
  const rows = list ? alerts : alerts.slice(0, 1);
  const more = alerts.length - 1;
  return (
    <div className={`alert-band alert-band-${alerts[0].kind}${fresh ? " fresh" : ""}`}>
      {rows.map((a, i) => (
        <div key={a.id} className={`alert-row alert-${a.kind}`}>
          <span className="alert-text" title={a.text}>
            {ALERT_EMOJI[a.kind]} {a.text}
          </span>
          {!list && i === 0 && more > 0 && (
            <button
              className="alert-more"
              title={alerts
                .slice(1)
                .map((b) => `${ALERT_EMOJI[b.kind]} ${b.text}`)
                .join("\n")}
              onClick={stop(() => setOpen(true))}
            >
              +{more}
            </button>
          )}
          <button className="x" title="Dismiss" onClick={stop(() => onDismiss(a.id))}>
            ✕
          </button>
        </div>
      ))}
      {list && (
        <div className="alert-actions">
          <button className="alert-more" onClick={stop(() => setOpen(false))}>
            Fewer
          </button>
          <button
            className="alert-more"
            onClick={stop(() => {
              alerts.forEach((a) => onDismiss(a.id));
              setOpen(false);
            })}
          >
            Dismiss all
          </button>
        </div>
      )}
    </div>
  );
}

/** Progress items shown on a job line before "+N". */
const PROGRESS_SHOWN = 3;

/** A background task on a card: its name and how long it has run, the progress it reports
 * (SPEC §3.18), and the latest line of its output. Minimised (`brief`): the name only. */
function JobLine({
  job,
  name,
  now,
  expanded,
  brief = false,
}: {
  job: Background;
  name: string;
  now: number;
  expanded: boolean;
  brief?: boolean;
}) {
  const timing = brief ? null : jobTiming(job, now);
  const shown = expanded ? job.progress : job.progress.slice(0, PROGRESS_SHOWN);
  const more = job.progress.length - shown.length;
  return (
    <div className={`card-job${expanded ? " full" : ""}`}>
      <span className="card-job-head">
        <span className="card-job-name">{name}</span>
        {timing && <span className="card-job-time">{timing}</span>}
      </span>
      {!brief &&
        shown.map((p, i) => (
          <span key={p.item ?? i} className={`card-job-progress state-${p.state ?? "running"}`}>
            {p.item && <span className="card-job-item">{p.item}</span>}
            <span
              className="card-job-steps"
              title={p.until ? `Expected to finish ${clock(p.until)}` : undefined}
            >
              {progressLine(p, now, job.seen_at)}
            </span>
            {p.url && (
              <button
                className="card-job-link"
                title={p.url}
                onClick={stop(() => void openUrl(p.url!))}
              >
                ↗
              </button>
            )}
          </span>
        ))}
      {!brief && more > 0 && (
        <span className="card-job-progress muted" data-more="">
          +{more} more
        </span>
      )}
      {!brief && job.last_line && <span className="card-job-output">{job.last_line}</span>}
    </div>
  );
}
