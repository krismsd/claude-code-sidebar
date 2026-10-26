import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { Project, Session } from "../lib/types";
import { SessionCard } from "./SessionCard";
import { TasksPanel } from "./TasksPanel";
import { arrange, projectStatus, reorder } from "../lib/view";
import { elapsed, sessionName } from "../lib/format";
import {
  DRAG_TYPE,
  draggedProject,
  draggedSession,
  dropPlace,
  endProjectDrag,
  endSessionDrag,
  startProjectDrag,
  startSessionDrag,
} from "../lib/drag";

export interface CardHandlers {
  now: number;
  fresh: Set<string>;
  onActivate: (s: Session) => void;
  onDismissAlert: (s: Session, alertId: number) => void;
  onAnswer: (s: Session, allow: boolean) => void;
  onHover: (s: Session, el: HTMLElement | null) => void;
  onMenu: (s: Session, x: number, y: number) => void;
  onMinimize: (s: Session, minimized: boolean) => void;
}

interface Props {
  /** null for the "Other" section (sessions whose project isn't resolved yet). */
  project: Project | null;
  sessions: Session[];
  cards: CardHandlers;
  /** projects.toml has [project.tasks] for this project. */
  hasTasks: boolean;
  onSetupTasks: () => void;
  onMessage: (msg: string) => void;
  onNewSession: (root: string) => void;
  onProjectAction: (root: string, action: "folder" | "vscode") => void;
  onCloseProject: (root: string) => void;
  /** Minimise (or expand) all the project's session cards. */
  onMinimizeAll: (ids: string[], minimized: boolean) => void;
  /** Delete a closed session's card for good. */
  onDeleteSession: (s: Session) => void;
  /** Drag-to-reorder: a project was dropped just before or after this section. */
  onDropProject: (draggedRoot: string, ontoRoot: string, place: "before" | "after") => void;
  /** Put the project in the drawer. */
  onShelve: (root: string) => void;
  /** Session ids in the user's dragged order. */
  sessionOrder: string[];
  onReorderSessions: (root: string, ids: string[]) => void;
}

/** Roughly the project menu's height, to decide whether it fits below the header. */
const MENU_HEIGHT = 150;
/** Height of one entry in the closed-sessions menu, for the same decision. */
const CLOSED_ITEM_HEIGHT = 28;

export function ProjectSection({
  project,
  sessions,
  cards,
  onNewSession,
  onProjectAction,
  onCloseProject,
  onDropProject,
  onShelve,
  sessionOrder,
  onReorderSessions,
  onDeleteSession,
  onMinimizeAll,
  hasTasks,
  onSetupTasks,
  onMessage,
}: Props) {
  const [showTasks, setShowTasks] = useState(false);
  /** Open menu: the project's actions (⋯) or its closed sessions ("N closed"). */
  const [menu, setMenu] = useState<"actions" | "closed" | false>(false);
  /** Open the menu upwards when there's no room under the header (the last project). */
  const [menuUp, setMenuUp] = useState(false);
  const [confirmClose, setConfirmClose] = useState(false);
  const [dropAt, setDropAt] = useState<"before" | "after" | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // The menu closes on a click outside it or Escape (not on mouse-leave, which is twitchy).
  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenu(false);
    // Clicking anywhere outside the sidebar (another window) closes it too.
    const onBlur = () => setMenu(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  }, [menu]);
  const { visible, closed } = arrange(sessions, sessionOrder);
  /** Open a header menu, upwards when there's no room under the header (the last project). */
  const openMenu = (e: React.MouseEvent, which: "actions" | "closed", height: number) => {
    const list = e.currentTarget.closest(".cards")?.getBoundingClientRect();
    const head = e.currentTarget.getBoundingClientRect();
    setMenuUp(!!list && head.bottom + height > list.bottom && head.top > height);
    setMenu(menu === which ? false : which);
  };
  const [cardDrop, setCardDrop] = useState<{ id: string; place: "before" | "after" } | null>(null);
  // Attention (dots, flashes) is shown on the sessions, not on their project.
  const status = projectStatus(sessions, cards.fresh);
  const root = project?.root;
  const color = project?.color ?? "#888";

  return (
    <section
      className={`project${dropAt ? ` drop-${dropAt}` : ""}`}
      style={{ "--project": color } as CSSProperties}
      onDragOver={(e) => {
        if (!root || !e.dataTransfer.types.includes(DRAG_TYPE)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        if (draggedProject() === root) return setDropAt(null);
        // Upper half: it goes above this project; lower half: below.
        const r = e.currentTarget.getBoundingClientRect();
        setDropAt(e.clientY < r.top + r.height / 2 ? "before" : "after");
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setDropAt(null);
      }}
      onDrop={(e) => {
        const place = dropAt;
        setDropAt(null);
        const dragged = e.dataTransfer.getData(DRAG_TYPE);
        if (root && place && dragged && dragged !== root) onDropProject(dragged, root, place);
      }}
    >
      <header
        className="project-head"
        draggable={!!root}
        onDragStart={(e) => root && startProjectDrag(e, root)}
        onDragEnd={endProjectDrag}
        title={root ? `${root}\nDrag to reorder, or onto the drawer to put it away` : undefined}
      >
        <span className="project-name">
          {project?.emoji && <span className="emoji">{project.emoji}</span>}
          {project?.name ?? "Other"}
          {project?.label && <span className="label-tag">{project.label}</span>}
        </span>
        {closed.length > 0 && (
          <button
            className="closed-link"
            title="Closed sessions: pick one to reopen it"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => openMenu(e, "closed", closed.length * CLOSED_ITEM_HEIGHT + 10)}
          >
            {closed.length} closed
          </button>
        )}
        {root && (
          <>
            <button className="icon" title="New session here" onClick={() => onNewSession(root)}>
              +
            </button>
            <button
              className="icon"
              title="Project actions"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={(e) => openMenu(e, "actions", MENU_HEIGHT)}
            >
              ⋯
            </button>
          </>
        )}
      </header>
      {menu === "closed" && (
        <div className={`project-menu closed-menu${menuUp ? " up" : ""}`} ref={menuRef}>
          {closed.map((s) => (
            <div className="closed-item" key={s.session_id}>
              <button
                className="closed-open"
                title="Reopen"
                onClick={() => (cards.onActivate(s), setMenu(false))}
              >
                <span className="closed-name">{sessionName(s)}</span>
                <span className="muted small">{elapsed(s.status_since, cards.now)}</span>
              </button>
              <button
                className="closed-delete"
                title="Delete it from the sidebar"
                onClick={() => onDeleteSession(s)}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      )}
      {menu === "actions" && root && (
        <div className={`project-menu${menuUp ? " up" : ""}`} ref={menuRef}>
          {hasTasks ? (
            <button onClick={() => (setShowTasks(true), setMenu(false))}>Tasks…</button>
          ) : (
            <button
              onClick={() => (onSetupTasks(), setMenu(false))}
              title="Add [project.tasks] for this project in projects.toml"
            >
              Set up tasks…
            </button>
          )}
          <button onClick={() => (onShelve(root), setMenu(false))}>Put in drawer</button>
          {visible.length > 0 && (
            <button
              onClick={() => {
                const allMin = visible.every((v) => v.minimized);
                onMinimizeAll(
                  visible.map((v) => v.session_id),
                  !allMin,
                );
                setMenu(false);
              }}
            >
              {visible.every((v) => v.minimized) ? "Expand all" : "Minimise all"}
            </button>
          )}
          <button onClick={() => (onProjectAction(root, "folder"), setMenu(false))}>
            Open folder
          </button>
          <button onClick={() => (onProjectAction(root, "vscode"), setMenu(false))}>VS Code</button>
          <button
            className={confirmClose ? "danger" : ""}
            disabled={status.open > 0}
            title={status.open > 0 ? "Remove its running sessions first" : undefined}
            onClick={() => {
              if (!confirmClose) {
                setConfirmClose(true);
                setTimeout(() => setConfirmClose(false), 3000);
              } else {
                setConfirmClose(false);
                setMenu(false);
                onCloseProject(root);
              }
            }}
          >
            {confirmClose
              ? `Really close? Removes ${sessions.length} closed session${sessions.length === 1 ? "" : "s"}`
              : "Close project"}
          </button>
        </div>
      )}
      {showTasks && root && (
        <TasksPanel root={root} onClose={() => setShowTasks(false)} onMessage={onMessage} />
      )}
      <div className="project-cards">
        {visible.map((s) => (
          // Drag a card onto another in the same project to reorder them; a line shows where
          // it will go.
          <div
            key={s.session_id}
            className={`card-slot${cardDrop?.id === s.session_id ? ` drop-${cardDrop.place}` : ""}`}
            draggable={!!root}
            onDragStart={(e) => root && startSessionDrag(e, root, s.session_id)}
            onDragEnd={() => (endSessionDrag(), setCardDrop(null))}
            onDragOver={(e) => {
              const d = draggedSession();
              if (!d || d.root !== root) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              setCardDrop(d.id === s.session_id ? null : { id: s.session_id, place: dropPlace(e) });
            }}
            onDragLeave={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node)) setCardDrop(null);
            }}
            onDrop={(e) => {
              const d = draggedSession();
              const place = cardDrop?.place;
              setCardDrop(null);
              if (!root || !d || d.root !== root || !place || d.id === s.session_id) return;
              e.preventDefault();
              e.stopPropagation();
              const ids = visible.map((v) => v.session_id);
              onReorderSessions(root, reorder(ids, d.id, s.session_id, place));
            }}
          >
            <SessionCard session={s} {...cards} />
          </div>
        ))}
        {visible.length === 0 && (
          <div className="muted small empty-project">No sessions. + starts one.</div>
        )}
      </div>
    </section>
  );
}
