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
      {expanded && (
        <div className="card-more">
          {s.last_prompt && (
            <div>
              <span className="muted">You: </span>
              {s.last_prompt}
            </div>
          )}
          {s.alerts.length > 1 &&
            s.alerts.map((a) => (
              <div key={a.id}>
                {ALERT_EMOJI[a.kind]} {a.text}
              </div>
            ))}
          <div className="muted small">
            {[
              s.tag && `#${s.tag}`,
              s.model,
              s.permission_mode,
              s.shell_cwd && s.shell_cwd !== s.cwd && `shell in ${s.shell_cwd}`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
          <div className="muted small">Right-click for more</div>
        </div>
      )}
    </article>
  );
}
