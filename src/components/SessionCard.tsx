import { useRef, type CSSProperties } from "react";
import type { Background, PullRequest, Session } from "../lib/types";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cacheState, dormant, readyStale, tokens } from "../lib/view";
import { ALERT_EMOJI, STATUS_LABEL, closedLabel, elapsed, sessionName } from "../lib/format";

interface Props {
  session: Session;
  now: number;
  /** Alert/permission keys that just appeared (see App useFresh). */
  fresh: Set<string>;
  onActivate: (s: Session) => void;
  onDismissAlert: (s: Session, alertId: number) => void;
  onAnswer: (s: Session, allow: boolean) => void;
  /** Hover in/out, with the card element so the expanded copy can be drawn over it. */
  onHover: (s: Session, el: HTMLElement | null) => void;
  /** Right-click: the session menu at the cursor. */
  onMenu: (s: Session, x: number, y: number) => void;
  /** Minimise the card to its title bar, or expand it. */
  onMinimize: (s: Session, minimized: boolean) => void;
  /** The hover copy: full recap and details below the usual content. */
  expanded?: boolean;
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
  const latest = [...shown].sort((a, b) => b.at - a.at)[0];
  // A dormant session (ended for sitting idle) looks and acts like a ready one.
  const asleep = dormant(s);
  const closed = s.status === "closed" && !asleep;
  const status = asleep ? "idle" : s.status;
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
    services.length > 0 ||
    agents > 0 ||
    (cache !== null && !cache.warm) ||
    s.compacted !== null ||
    shown.length > 1;
  const label = [s.project?.worktree && `⑂ ${s.project.worktree}`, s.project?.branch]
    .filter(Boolean)
    .join(" · ");

  return (
    <article
      className={`card status-${status}${flashing ? " flash" : ""}${closed ? " is-closed" : ""}${folded ? " minimized" : ""}`}
      style={{ "--project": s.project?.color ?? "#888" } as CSSProperties}
      onClick={() => onActivate(s)}
      // The title bar and PR links are controls (the fold toggle, opening a PR), so only the
      // rest of the body opens the hover copy, which would cover them; a minimised card, all
      // title bar, never does.
      onMouseOver={(e) => {
        const body = !folded && !(e.target as Element).closest(".card-head, .card-footer");
        if (body !== overBody.current) {
          overBody.current = body;
          onHover(s, body ? e.currentTarget : null);
        }
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
        <span className="card-title">
          <span className="card-name">{sessionName(s)}</span>
          {label && <span className="card-branch"> · {label}</span>}
        </span>
        <span
          className={`chip card-status-block chip-${status}${status === "idle" && readyStale(s, now) ? " stale" : ""}`}
          title={cache ? cacheDetail(cache) : undefined}
        >
          {closed ? closedLabel(s) : STATUS_LABEL[status]} · {elapsed(s.status_since, now)}
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
          {latest && (
            <div className={`card-alert alert-${latest.kind}`}>
              <span className="card-alert-text">
                {ALERT_EMOJI[latest.kind]} {latest.text}
              </span>
              <button
                className="x"
                title="Dismiss"
                onClick={stop(() => onDismissAlert(s, latest.id))}
              >
                ✕
              </button>
            </div>
          )}
          {/* Claude's last answer as written: the start on the card, all of it on hover. The
          question it ended on, if any, is pinned below so it's never clipped. */}
          {(s.last_answer ?? s.recap) && (
            <div className={`card-recap${expanded ? " full" : ""}`}>
              {/* No blank lines: they only take room (saved answers may still have them). */}
              {(s.last_answer ?? s.recap ?? "")
                .split("\n")
                .filter((l) => l.trim())
                .join("\n")}
            </div>
          )}
          {s.question && !expanded && <div className="card-question">{s.question}</div>}
          {expanded && (
            <div className="card-more">
              {s.last_prompt && (
                <div>
                  <span className="muted">You: </span>
                  {s.last_prompt}
                </div>
              )}
              {services.map((b, i) => (
                <JobLine key={b.id || `bg${i}`} job={b} name={`⧗ ${jobName(b)}`} expanded />
              ))}
              {shown.length > 1 &&
                shown.map((a) => (
                  <div key={a.id}>
                    {ALERT_EMOJI[a.kind]} {a.text}
                  </div>
                ))}
              {cache && <div className="muted small">{cacheDetail(cache)}</div>}
              <div className="muted small">
                {[s.tag && `#${s.tag}`, s.model, s.permission_mode].filter(Boolean).join(" · ")}
              </div>
              <div className="muted small right-click-hint">Right-click for more</div>
            </div>
          )}
        </>
      )}
      {/* Footer bands, kept when the card is minimised: jobs Claude will act on when they
      finish (watchers, background agents), then PRs the session opened. */}
      {jobs.length + prs.length + (statusBand ? 1 : 0) > 0 && (
        <footer className="card-footer">
          {jobs.length > 0 && (
            <div className="card-jobs" title={jobs.map(jobName).join("\n")}>
              {jobs.map((b, i) => (
                <JobLine
                  key={b.id || i}
                  job={folded ? { ...b, last_line: null } : b}
                  name={`${b.agent ? "🤖" : "⏱"} ${jobName(b)}`}
                  expanded={expanded}
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
          {statusBand && (
            <div className="card-status">
              {services.length > 0 && (
                <span
                  title={`${plural(services.length, "background task")}:\n${services.map(jobName).join("\n")}`}
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
              <span className="grow" />
              {shown.length > 1 &&
                shown.map((a) => (
                  <button
                    key={a.id}
                    className={`badge badge-${a.kind}${isFresh(a.id) ? " fresh" : ""}`}
                    title={`${a.text}\n(click to dismiss)`}
                    onClick={stop(() => onDismissAlert(s, a.id))}
                  >
                    {ALERT_EMOJI[a.kind]}
                  </button>
                ))}
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

/** A background task on a card: its name, and the latest line of its output. */
function JobLine({ job, name, expanded }: { job: Background; name: string; expanded: boolean }) {
  return (
    <div className={`card-job${expanded ? " full" : ""}`}>
      <span className="card-job-name">{name}</span>
      {job.last_line && <span className="card-job-output">{job.last_line}</span>}
    </div>
  );
}
