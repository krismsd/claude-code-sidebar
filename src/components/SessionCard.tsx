import type { CSSProperties } from "react";
import type { Session } from "../lib/types";
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
  /** Hover in/out, with the card element so the flyout can sit next to it. */
  onHover: (s: Session, el: HTMLElement | null) => void;
}

const stop =
  (f: () => void) =>
  (e: { stopPropagation: () => void }): void => {
    e.stopPropagation();
    f();
  };

/** Compact session card. Its height never changes on hover: details live in the flyout. */
export function SessionCard({
  session: s,
  now,
  fresh,
  onActivate,
  onDismissAlert,
  onAnswer,
  onStar,
  onHover,
}: Props) {
  const isFresh = (alertId: number) => fresh.has(`${s.session_id}:${alertId}`);
  const flashing =
    s.alerts.some((a) => isFresh(a.id)) ||
    (s.pending_permission !== null &&
      fresh.has(`${s.session_id}:perm:${s.pending_permission.since}`));
  const latest = [...s.alerts].sort((a, b) => b.at - a.at)[0];
  const closed = s.status === "closed";
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
      className={`card status-${s.status}${flashing ? " flash" : ""}${closed ? " is-closed" : ""}`}
      style={{ "--project": s.project?.color ?? "#888" } as CSSProperties}
      onClick={() => onActivate(s)}
      onMouseEnter={(e) => onHover(s, e.currentTarget)}
      onMouseLeave={() => onHover(s, null)}
    >
      <header className="card-head">
        <button
          className={`star${s.starred ? " on" : ""}`}
          title={s.starred ? "Unstar" : "Star: keep it here, even when closed"}
          onClick={stop(() => onStar(s))}
        >
          {s.starred ? "★" : "☆"}
        </button>
        <span
          className="card-title"
          title={label ? `${sessionName(s)} · ${label}` : sessionName(s)}
        >
          <span className="card-name">{sessionName(s)}</span>
          {label && <span className="card-branch"> · {label}</span>}
        </span>
        <span className={`chip chip-${s.status}`}>
          {closed ? closedLabel(s) : STATUS_LABEL[s.status]} · {elapsed(s.status_since, now)}
        </span>
      </header>
      {!closed && s.tag === null && (
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
        <div className={`card-recap${s.recap_haiku ? "" : " instant"}`}>
          {s.recap_haiku ?? s.recap}
        </div>
      )}
      {(s.alerts.length > 1 || s.subagents_running + s.background_running > 0) && (
        <div className="card-badges">
          {s.alerts.length > 1 &&
            s.alerts.map((a) => (
              <button
                key={a.id}
                className={`badge badge-${a.kind}${isFresh(a.id) ? " fresh" : ""}`}
                title={`${a.text}\n(click to dismiss)`}
                onClick={stop(() => onDismissAlert(s, a.id))}
              >
                {ALERT_EMOJI[a.kind]}
              </button>
            ))}
          {s.subagents_running + s.background_running > 0 && (
            <span className="muted small">
              ⧗ {s.subagents_running + s.background_running} in background
            </span>
          )}
        </div>
      )}
    </article>
  );
}
