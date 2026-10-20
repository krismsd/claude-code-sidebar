import { useEffect, useRef, useState } from "react";
import type { Session } from "../lib/types";
import { sessionName } from "../lib/format";

interface Props {
  session: Session;
  x: number;
  y: number;
  onClose: () => void;
  onRename: (s: Session, name: string) => void;
  onEnd: (s: Session) => void;
  onRemove: (s: Session) => void;
  onMessage: (msg: string) => void;
}

/** Right-click menu for a session: rename, copy its ID, end it or remove it. */
export function SessionMenu({
  session: s,
  x,
  y,
  onClose,
  onRename,
  onEnd,
  onRemove,
  onMessage,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(s.user_title ?? sessionName(s));
  const [confirmEnd, setConfirmEnd] = useState(false);
  const closed = s.status === "closed";

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
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
              navigator.clipboard
                .writeText(s.session_id)
                .then(() => onMessage(`Copied ${s.session_id}`))
                .catch((e) => onMessage(String(e)));
              onClose();
            }}
          >
            Copy session ID
          </button>
          {!closed && (
            <button
              className={confirmEnd ? "danger" : ""}
              disabled={s.pid === null}
              title={
                s.pid === null ? "Started with plain claude: its process isn't known" : undefined
              }
              onClick={() => {
                if (confirmEnd) {
                  onEnd(s);
                  onClose();
                } else setConfirmEnd(true);
              }}
            >
              {confirmEnd ? "Really end this session?" : "End session"}
            </button>
          )}
          {closed && (
            <button
              onClick={() => {
                onRemove(s);
                onClose();
              }}
            >
              Remove from sidebar
            </button>
          )}
        </>
      )}
    </div>
  );
}
