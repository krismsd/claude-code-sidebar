import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { Project, Session } from "../lib/types";
import { SessionCard } from "./SessionCard";
import { TasksPanel } from "./TasksPanel";
import { arrange, projectStatus } from "../lib/view";

export interface CardHandlers {
  now: number;
  fresh: Set<string>;
  onActivate: (s: Session) => void;
  onDismissAlert: (s: Session, alertId: number) => void;
  onAnswer: (s: Session, allow: boolean) => void;
  onStar: (s: Session) => void;
  onHover: (s: Session, el: HTMLElement | null) => void;
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
  /** Drag-to-reorder: a project header was dropped onto this section. */
  onDropProject: (draggedRoot: string, ontoRoot: string) => void;
}

export function ProjectSection({
  project,
  sessions,
  cards,
  onNewSession,
  onProjectAction,
  onCloseProject,
  onDropProject,
  hasTasks,
  onSetupTasks,
  onMessage,
}: Props) {
  const [showTasks, setShowTasks] = useState(false);
  const [showClosed, setShowClosed] = useState(false);
  const [menu, setMenu] = useState(false);
  const [confirmClose, setConfirmClose] = useState(false);
  const [dropTarget, setDropTarget] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // The menu closes on a click outside it or Escape (not on mouse-leave, which is twitchy).
  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenu(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menu]);
  const { visible, closed } = arrange(sessions);
  const status = projectStatus(sessions, cards.fresh);
  const flashing = status.flashing;
  const root = project?.root;
  const color = project?.color ?? "#888";

  return (
    <section
      className={`project${dropTarget ? " drop-target" : ""}`}
      style={{ "--project": color } as CSSProperties}
      onDragOver={(e) => {
        if (root && e.dataTransfer.types.includes("application/x-cc-project")) {
          e.preventDefault();
          setDropTarget(true);
        }
      }}
      onDragLeave={() => setDropTarget(false)}
      onDrop={(e) => {
        setDropTarget(false);
        const dragged = e.dataTransfer.getData("application/x-cc-project");
        if (root && dragged && dragged !== root) onDropProject(dragged, root);
      }}
    >
      <header
        className={`project-head${flashing ? " flash" : ""}`}
        draggable={!!root}
        onDragStart={(e) => root && e.dataTransfer.setData("application/x-cc-project", root)}
        title={root ? `${root}\nDrag to reorder` : undefined}
      >
        <span className={`dot ${status.state}`} />
        <span className="project-name">
          {project?.emoji && <span className="emoji">{project.emoji}</span>}
          {project?.name ?? "Other"}
          {project?.label && <span className="label-tag">{project.label}</span>}
        </span>
        <span className="muted small">{status.open > 0 ? `${status.open} open` : ""}</span>
        {root && (
          <>
            <button className="icon" title="New session here" onClick={() => onNewSession(root)}>
              +
            </button>
            <button
              className="icon"
              title="Project actions"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => setMenu(!menu)}
            >
              ⋯
            </button>
          </>
        )}
      </header>
      {menu && root && (
        <div className="project-menu" ref={menuRef}>
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
          <button onClick={() => (onProjectAction(root, "folder"), setMenu(false))}>
            Open folder
          </button>
          <button onClick={() => (onProjectAction(root, "vscode"), setMenu(false))}>VS Code</button>
          <button
            className={confirmClose ? "danger" : ""}
            disabled={status.open > 0}
            title={status.open > 0 ? "End or close its sessions first" : undefined}
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
          <SessionCard key={s.session_id} session={s} {...cards} />
        ))}
        {visible.length === 0 && closed.length === 0 && (
          <div className="muted small empty-project">No sessions. + starts one.</div>
        )}
        {closed.length > 0 && (
          <button className="closed-toggle" onClick={() => setShowClosed(!showClosed)}>
            {showClosed ? "▾" : "▸"} {closed.length} closed
          </button>
        )}
        {showClosed && closed.map((s) => <SessionCard key={s.session_id} session={s} {...cards} />)}
      </div>
    </section>
  );
}
