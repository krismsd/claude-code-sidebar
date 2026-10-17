import { useState, type CSSProperties } from "react";
import type { Session, SessionAction } from "../lib/types";
import { ALERT_EMOJI } from "../lib/format";
import { flyoutPosition } from "../lib/view";

interface Props {
  session: Session;
  /** The hovered card's position (viewport coordinates). */
  anchor: DOMRect;
  onEnter: () => void;
  onLeave: () => void;
  onAction: (s: Session, action: SessionAction) => void;
  onDismiss: (s: Session) => void;
}

/** Card details, floating next to the hovered card without moving any layout (SPEC §3.12).
 * Opens below the card if there's room, otherwise above it. */
export function Flyout({ session: s, anchor, onEnter, onLeave, onAction, onDismiss }: Props) {
  const [confirmEnd, setConfirmEnd] = useState(false);
  const closed = s.status === "closed";
  const style: CSSProperties = flyoutPosition(anchor, window.innerHeight);

  return (
    <div
      className="flyout"
      style={{ ...style, borderColor: s.project?.color ?? "var(--border)" }}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      {(s.recap_haiku ?? s.recap) && <div className="flyout-recap">{s.recap_haiku ?? s.recap}</div>}
      {s.recap_detail && <div className="recap-detail">{s.recap_detail}</div>}
      {s.last_prompt && (
        <div>
          <span className="muted">You: </span>
          {s.last_prompt}
        </div>
      )}
      {s.alerts.map((a) => (
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
      <div className="actions">
        {!closed && s.pid !== null && (
          <button
            className={confirmEnd ? "danger" : ""}
            onClick={() => {
              if (confirmEnd) {
                setConfirmEnd(false);
                onAction(s, "end");
              } else {
                setConfirmEnd(true);
                setTimeout(() => setConfirmEnd(false), 3000);
              }
            }}
          >
            {confirmEnd ? "Really end?" : "End"}
          </button>
        )}
        <button onClick={() => onAction(s, "folder")}>Folder</button>
        <button onClick={() => onAction(s, "vscode")}>VS Code</button>
        {closed && <button onClick={() => onDismiss(s)}>Remove</button>}
      </div>
    </div>
  );
}
