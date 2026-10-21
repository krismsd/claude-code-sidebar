import { useEffect, useRef, useState } from "react";
import type { Session } from "../lib/types";
import { sessionName } from "../lib/format";

interface Props {
  session: Session;
  x: number;
  y: number;
  onClose: () => void;
  onRename: (s: Session, name: string) => void;
  /** Minimise the card to its title bar, or expand it. */
  onMinimize: (s: Session, minimized: boolean) => void;
  /** Move it to the closed section, ending it (and what it runs) if it's running. */
  onRemove: (s: Session) => void;
  /** Kill a stuck session and everything it started; it stays in the list. */
  onForceEnd: (s: Session) => void;
  /** Delete a removed session's card for good. */
  onDelete: (s: Session) => void;
  /** Hide it (detach every terminal) or open it in a Windows Terminal tab (tmux sessions). */
  onAction: (s: Session, action: "hide" | "terminal") => void;
  onMessage: (msg: string) => void;
}

/** Right-click menu for a session: rename, copy its ID, show or hide it, remove it (asking
 * first if that ends work in progress), force-end a stuck one, delete a removed one. */
export function SessionMenu({
  session: s,
  x,
  y,
  onClose,
  onRename,
  onMinimize,
  onRemove,
  onForceEnd,
  onDelete,
  onAction,
  onMessage,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(s.user_title ?? sessionName(s));
  const [confirm, setConfirm] = useState<"remove" | "force" | null>(null);
  const closed = s.status === "closed";
  const running = !closed;
  const removed = s.removed === true;
  // Removing a running session ends it: say what that stops, and ask first.
  const tasks = s.background.length;
  const busy = !running
    ? ""
    : tasks > 0
      ? `Ends it and ${tasks === 1 ? "its background task" : `its ${tasks} background tasks`}`
      : s.status === "working"
        ? "It's still working"
        : s.status === "needs_permission" || s.status === "needs_input"
          ? "It's waiting for you"
          : "";

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // Clicking anywhere outside the sidebar (another window) closes it too.
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  // Keep the menu on screen.
  const style = {
    left: Math.min(x, window.innerWidth - 230),
    top: Math.min(y, window.innerHeight - 170),
  };

  return (
    <div className="session-menu" ref={ref} style={style}>
      {renaming ? (
        <form
          className="rename"
          onSubmit={(e) => {
            e.preventDefault();
            onRename(s, name);
            onClose();
          }}
        >
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onFocus={(e) => e.target.select()}
            spellCheck={false}
          />
          <div className="muted small">
            {closed
              ? "Also shown in Claude Code's /resume list."
              : "Written to Claude Code when the session closes."}{" "}
            Empty clears it.
          </div>
        </form>
      ) : (
        <>
          <button onClick={() => setRenaming(true)}>Rename…</button>
          <button
            onClick={() => {
              onMinimize(s, !s.minimized);
              onClose();
            }}
          >
            {s.minimized ? "Expand" : "Minimise"}
          </button>
          {!closed && s.tmux && s.attached && (
            <button
              title="Close its terminal; Claude keeps running in the background"
              onClick={() => {
                onAction(s, "hide");
                onClose();
              }}
            >
              Hide (keep running)
            </button>
          )}
          {!closed && s.tmux && (
            <button
              onClick={() => {
                onAction(s, "terminal");
                onClose();
              }}
            >
              Open in Windows Terminal
            </button>
          )}
          <button
            onClick={() => {
              navigator.clipboard
                .writeText(s.session_id)
                .then(() => onMessage(`Copied ${s.session_id}`))
                .catch((e) => onMessage(String(e)));
              onClose();
            }}
          >
            Copy session ID
          </button>
          {!removed && (
            <button
              className={confirm === "remove" ? "danger" : ""}
              title="Move it to the closed section, ending it if it's still running"
              onClick={() => {
                if (busy && confirm !== "remove") return setConfirm("remove");
                onRemove(s);
                onClose();
              }}
            >
              {confirm === "remove" ? `Remove? ${busy}` : "Remove"}
            </button>
          )}
          {running && (
            <button
              className={confirm === "force" ? "danger" : ""}
              disabled={s.pid === null}
              title={
                s.pid === null
                  ? "Started with plain claude: its process isn't known"
                  : "For a stuck session: kill it and everything it started now. It stays in the list."
              }
              onClick={() => {
                if (confirm !== "force") return setConfirm("force");
                onForceEnd(s);
                onClose();
              }}
            >
              {confirm === "force" ? "Kill it now?" : "Force end"}
            </button>
          )}
          {removed && closed && (
            <button
              onClick={() => {
                onDelete(s);
                onClose();
              }}
            >
              Delete
            </button>
          )}
        </>
      )}
    </div>
  );
}
