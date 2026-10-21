import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Background, Session } from "../lib/types";
import { dormant } from "../lib/view";
import { ALERT_EMOJI, STATUS_LABEL, closedLabel, elapsed, sessionName } from "../lib/format";

interface Props {
  session: Session;
  now: number;
  /** Alert/permission keys that just appeared (see App useFresh). */
  fresh: Set<string>;
  onActivate: (s: Session) => void;
  onDismissAlert: (s: Session, alertId: number) => void;
  onAnswer: (s: Session, allow: boolean) => void;
  onStar: (s: Session) => void;
  /** Hover in/out, with the card element so the expanded copy can be drawn over it. */
  onHover: (s: Session, el: HTMLElement | null) => void;
  /** Right-click: the session menu at the cursor. */
  onMenu: (s: Session, x: number, y: number) => void;
  /** The hover copy: full recap and details below the usual content. */
  expanded?: boolean;
}

const stop =
  (f: () => void) =>
  (e: { stopPropagation: () => void }): void => {
    e.stopPropagation();
    f();
  };

const HOUR = 3_600_000;

/** A "ready" pill fades from green to grey between 1 and 2 hours: still ready, but old news. */
function readyFade(s: Session, now: number): CSSProperties | undefined {
  if (s.status !== "idle" && !dormant(s)) return undefined;
  const grey = Math.min(1, Math.max(0, (now - s.status_since - HOUR) / HOUR));
  if (grey === 0) return undefined;
  return {
    background: `color-mix(in srgb, var(--grey) ${Math.round(grey * 100)}%, var(--green))`,
  };
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
  onStar,
  onHover,
  onMenu,
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
  const activity =
    s.status === "needs_permission" && s.pending_permission
      ? `Wants to: ${s.pending_permission.summary}`
      : s.status === "working" || s.status === "needs_input"
        ? (s.current_tool ?? (s.status === "working" ? "Thinking…" : null))
        : null;
  const label = [s.project?.worktree && `⑂ ${s.project.worktree}`, s.project?.branch]
    .filter(Boolean)
    .join(" · ");

  return (
    <article
      className={`card status-${status}${flashing ? " flash" : ""}${closed ? " is-closed" : ""}`}
      style={{ "--project": s.project?.color ?? "#888" } as CSSProperties}
      onClick={() => onActivate(s)}
      onMouseEnter={(e) => onHover(s, e.currentTarget)}
      onMouseLeave={() => onHover(s, null)}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(s, e.clientX, e.clientY);
      }}
    >
      <header className="card-head">
        <button
          className={`star${s.starred ? " on" : ""}`}
          title={s.starred ? "Unstar" : "Star: keep it here, even when closed"}
          onClick={stop(() => onStar(s))}
        >
          {s.starred ? "★" : "☆"}
        </button>
        <span className="card-title">
          <span className="card-name">{sessionName(s)}</span>
          {label && <span className="card-branch"> · {label}</span>}
        </span>
        <span className={`chip chip-${status}`} style={readyFade(s, now)}>
          {closed ? closedLabel(s) : STATUS_LABEL[status]} · {elapsed(s.status_since, now)}
        </span>
      </header>
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
          <button className="x" title="Dismiss" onClick={stop(() => onDismissAlert(s, latest.id))}>
            ✕
          </button>
        </div>
      )}
      {(s.recap_haiku ?? s.recap) && (
        <div className={`card-recap${s.recap_haiku ? "" : " instant"}${expanded ? " full" : ""}`}>
          {s.recap_haiku ? (
            <>
              <span className="recap-line">{s.recap_haiku}</span>
              {s.recap_detail && `\n${s.recap_detail}`}
            </>
          ) : (
            s.recap
          )}
        </div>
      )}
      {jobs.length > 0 && (
        <div className="card-jobs" title={jobs.map(jobName).join("\n")}>
          <span className="card-jobs-label">
            {jobs.length > 1 ? `Waiting on ${jobs.length} jobs` : "Waiting on"}
          </span>
          {jobs.map((b, i) => (
            <JobLine
              key={b.id || i}
              session={s}
              job={b}
              name={`${b.agent ? "🤖" : "⏱"} ${jobName(b)}`}
              expanded={expanded}
            />
          ))}
        </div>
      )}
      {(shown.length > 1 || agents + services.length > 0) && (
        <div className="card-badges">
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
          {agents > 0 && (
            <span className="running running-agents">🤖 {plural(agents, "agent")} running</span>
          )}
          {services.length > 0 && (
            <span
              className="running clickable"
              title={`${services.map(jobName).join("\n")}\n(click to show their output)`}
              onClick={(e) => {
                e.stopPropagation();
                services
                  .filter((b) => b.id)
                  .forEach((b) =>
                    invoke("open_job_output", { sessionId: s.session_id, taskId: b.id }),
                  );
              }}
            >
              ⧗ {plural(services.length, "background task")}
            </span>
          )}
        </div>
      )}
      {expanded && (
        <div className="card-more">
          {s.last_prompt && (
            <div>
              <span className="muted">You: </span>
              {s.last_prompt}
            </div>
          )}
          {services.map((b, i) => (
            <JobLine key={b.id || `bg${i}`} session={s} job={b} name={`⧗ ${jobName(b)}`} expanded />
          ))}
          {shown.length > 1 &&
            shown.map((a) => (
              <div key={a.id}>
                {ALERT_EMOJI[a.kind]} {a.text}
              </div>
            ))}
          <div className="muted small">
            {[s.tag && `#${s.tag}`, s.model, s.permission_mode].filter(Boolean).join(" · ")}
          </div>
          <div className="muted small right-click-hint">Right-click for more</div>
        </div>
      )}
    </article>
  );
}

/** A background task on a card: its name, its latest output line, and a link to what it
 * watches if its output (or command) has one. A click follows its whole output live in the
 * terminal window. */
function JobLine({
  session,
  job,
  name,
  expanded,
}: {
  session: Session;
  job: Background;
  name: string;
  expanded: boolean;
}) {
  const open = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (job.id) invoke("open_job_output", { sessionId: session.session_id, taskId: job.id });
  };
  return (
    <div
      className={`card-job${expanded ? " full" : ""}${job.id ? " clickable" : ""}`}
      title={job.id ? "Show its output" : undefined}
      onClick={open}
    >
      <span className="card-job-name">
        {name}
        {job.url && (
          <button
            className="card-job-link"
            title={job.url}
            onClick={(e) => {
              e.stopPropagation();
              openUrl(job.url!);
            }}
          >
            ↗
          </button>
        )}
      </span>
      {job.last_line && <span className="card-job-output">{job.last_line}</span>}
    </div>
  );
}
