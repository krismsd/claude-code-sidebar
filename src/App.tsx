import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Session, UiState } from "./lib/types";
import { ProjectSection, type CardHandlers } from "./components/ProjectSection";
import { SessionCard } from "./components/SessionCard";
import { SessionMenu } from "./components/SessionMenu";
import { AddProject } from "./components/AddProject";
import { Drawer } from "./components/Drawer";
import { Strip } from "./components/Strip";
import { UsageFooter } from "./components/UsageFooter";
import { WindowMenu } from "./components/WindowMenu";
import { dropIndex, freshKeys, groupSessions } from "./lib/view";
import { DRAG_TYPE } from "./lib/drag";
import "./App.css";

type Outcome = "focused" | "resumed" | "reopened" | "tab_not_found" | "untagged" | "failed";

const OUTCOME_MESSAGE: Partial<Record<Outcome, string>> = {
  resumed: "Resuming in a new tab…",
  reopened: "It was running in the background: opened in a new tab",
  tab_not_found: "Couldn't find its terminal tab (is it running outside Windows Terminal?)",
  untagged:
    "Started with plain `claude`, so the sidebar can't tell which tab it's in. Run /exit, then `claudes --continue`.",
  failed: "That didn't work. See the log for details.",
};

const FLASH_MS = 4000;
const HOVER_OPEN_MS = 300;
/** Between leaving one card and entering the next, so the overlay doesn't blink. */
const HOVER_CLOSE_MS = 60;

/** The hovered card drawn expanded, exactly over the original (SPEC §3.12). It ignores the
 * mouse, so hover still follows the cards underneath: moving below the original card's bottom
 * edge hovers the next card. Shifted up only if it would run off the bottom of the window.
 * Placed from the original card on every render, so it follows when cards above it change
 * height. */
function HoverCard({ anchor, children }: { anchor: HTMLElement; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const place = () => {
      const rect = anchor.getBoundingClientRect();
      const max = window.innerHeight - el.offsetHeight - 6;
      el.style.left = `${rect.left}px`;
      el.style.width = `${rect.width}px`;
      el.style.top = `${Math.max(6, Math.min(rect.top, max))}px`;
    };
    place();
    // Layout can also shift without a re-render of ours (fonts, a section's own state).
    const ro = new ResizeObserver(place);
    const list = anchor.closest(".cards");
    if (list) [...list.children].forEach((c) => ro.observe(c));
    return () => ro.disconnect();
  });
  return (
    <div ref={ref} className="hover-card">
      {children}
    </div>
  );
}

function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [intervalMs]);
  return now;
}

/** Alert/permission keys that appeared in the last few seconds (they flash). Whatever exists
 * on first load is old news. */
function useFresh(sessions: Session[] | null): Set<string> {
  const seen = useRef<Set<string> | null>(null);
  const [fresh, setFresh] = useState<Set<string>>(new Set());
  // Each batch's timer must survive later state updates (they arrive every few seconds), or a
  // key stays fresh for good and the card flashes again whenever it is redrawn.
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);
  useEffect(() => {
    if (sessions === null) return;
    const all = sessions.flatMap(freshKeys);
    if (seen.current === null) {
      seen.current = new Set(all);
      return;
    }
    const known = seen.current;
    const added = all.filter((k) => !known.has(k));
    if (added.length === 0) return;
    added.forEach((k) => known.add(k));
    setFresh((f) => new Set([...f, ...added]));
    timers.current.push(
      setTimeout(
        () => setFresh((f) => new Set([...f].filter((k) => !added.includes(k)))),
        FLASH_MS,
      ),
    );
  }, [sessions]);
  return fresh;
}

function App() {
  const [state, setState] = useState<UiState | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [windowMenu, setWindowMenu] = useState(false);
  const [hover, setHover] = useState<{ id: string; el: HTMLElement } | null>(null);
  const [draggingProject, setDraggingProject] = useState(false);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dragging = useRef(false);
  const now = useNow();
  const fresh = useFresh(state?.sessions ?? null);

  useEffect(() => {
    invoke<UiState>("get_state").then(setState);
    const unlisten = listen<UiState>("state", (e) => setState(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 7000);
    return () => clearTimeout(t);
  }, [toast]);

  const report = useCallback((e: unknown) => setToast(String(e)), []);

  // Hover: the expanded card opens after a short pause; while one is open, moving to another
  // card switches straight away.
  const onHover = (s: Session, el: HTMLElement | null) => {
    clearTimeout(openTimer.current);
    clearTimeout(closeTimer.current);
    if (el && dragging.current) return;
    if (!el) {
      closeTimer.current = setTimeout(() => setHover(null), HOVER_CLOSE_MS);
      return;
    }
    const next = { id: s.session_id, el };
    if (hover) setHover(next);
    else openTimer.current = setTimeout(() => setHover(next), HOVER_OPEN_MS);
  };

  // Our own right-click menu replaces the browser's (text boxes keep theirs).
  useEffect(() => {
    const block = (e: MouseEvent) => {
      if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
    };
    document.addEventListener("contextmenu", block);
    return () => document.removeEventListener("contextmenu", block);
  }, []);

  // While a project is being dragged, the drawer shows its drop zone even when empty.
  useEffect(() => {
    // No hover overlay while dragging, including one whose timer started as the drag began.
    const start = (e: DragEvent) => {
      dragging.current = true;
      clearTimeout(openTimer.current);
      setHover(null);
      if (e.dataTransfer?.types.includes(DRAG_TYPE)) setDraggingProject(true);
    };
    const end = () => {
      dragging.current = false;
      setDraggingProject(false);
    };
    document.addEventListener("dragstart", start);
    document.addEventListener("dragend", end);
    document.addEventListener("drop", end);
    return () => {
      document.removeEventListener("dragstart", start);
      document.removeEventListener("dragend", end);
      document.removeEventListener("drop", end);
    };
  }, []);

  const cards: CardHandlers = {
    now,
    fresh,
    onHover,
    onMenu: (s, x, y) => {
      setHover(null);
      setMenu({ id: s.session_id, x, y });
    },
    onActivate: async (s) => {
      setHover(null);
      try {
        const outcome = await invoke<Outcome>("activate_session", { sessionId: s.session_id });
        setToast(OUTCOME_MESSAGE[outcome] ?? null);
      } catch (e) {
        report(e);
      }
    },
    onDismissAlert: (s, alertId) =>
      invoke("dismiss_alert", { sessionId: s.session_id, alertId }).catch(report),
    onAnswer: (s, allow) =>
      invoke("answer_permission", { sessionId: s.session_id, allow }).catch(report),
    onStar: (s) => invoke("toggle_star", { sessionId: s.session_id }).catch(report),
  };

  if (!state) return <main className="sidebar" />;

  const setCollapsed = (collapsed: boolean) => {
    setHover(null);
    invoke("set_collapsed", { collapsed }).catch(report);
  };
  if (state.collapsed)
    return (
      <Strip
        projects={state.projects}
        sessions={state.sessions}
        fresh={fresh}
        leftEdge={state.left_edge}
        floating={state.floating}
        usage={state.usage}
        now={now}
        onExpand={() => setCollapsed(false)}
        onActivate={cards.onActivate}
      />
    );
  const drag = state.floating ? { "data-tauri-drag-region": true } : {};

  const { byRoot, other } = groupSessions(state);
  const openCount = state.sessions.filter((s) => s.status !== "closed").length;
  const hovered = hover && state.sessions.find((s) => s.session_id === hover.id);
  const menuSession = menu && state.sessions.find((s) => s.session_id === menu.id);

  const shelve = (root: string, shelved: boolean) => {
    setHover(null);
    invoke("set_shelved", { root, shelved }).catch(report);
  };
  /** A project dropped just before or after another, in the list or in the drawer: move it
   * there, into or out of the drawer as needed. */
  const placeProject = async (
    dragged: string,
    onto: string,
    place: "before" | "after",
    shelved: boolean,
  ) => {
    const from = state.projects.findIndex((t) => t.project.root === dragged);
    const at = state.projects.findIndex((t) => t.project.root === onto);
    const to = dropIndex(from, at, place);
    try {
      if (state.projects[from] && state.projects[from].shelved !== shelved)
        await invoke("set_shelved", { root: dragged, shelved });
      if (to !== from) await invoke("move_project", { root: dragged, to });
    } catch (e) {
      report(e);
    }
  };
  const sectionProps = {
    cards,
    onNewSession: (root: string) =>
      invoke("new_session", { path: root })
        .then(() => setToast("Starting a new session…"))
        .catch(report),
    onProjectAction: (root: string, action: "folder" | "vscode") =>
      invoke("project_action", { root, action }).catch(report),
    onCloseProject: (root: string) => invoke("close_project", { root }).catch(report),
    onSetupTasks: () => invoke("open_settings", { tab: "projects" }).catch(report),
    onMessage: setToast,
    onDropProject: (dragged: string, onto: string, place: "before" | "after") =>
      placeProject(dragged, onto, place, false),
    onShelve: (root: string) => shelve(root, true),
    onReorderSessions: (root: string, ids: string[]) =>
      invoke("set_session_order", { root, ids }).catch(report),
  };

  return (
    <main className="sidebar">
      <header className="sidebar-header" {...drag}>
        <span {...drag}>cc-sidebar</span>
        <span className="header-right">
          <span className="muted small">{openCount} open</span>
          <button className="plus" title="Add project" onClick={() => setCreating(!creating)}>
            +
          </button>
          <button
            className="plus"
            title="Window and settings"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={() => setWindowMenu(!windowMenu)}
          >
            ⚙
          </button>
          <button
            className="plus"
            title="Collapse to a status strip"
            onClick={() => setCollapsed(true)}
          >
            {state.left_edge ? "«" : "»"}
          </button>
        </span>
      </header>
      {windowMenu && (
        <WindowMenu
          floating={state.floating}
          monitor={state.monitor}
          onClose={() => setWindowMenu(false)}
          onError={report}
        />
      )}
      {creating && (
        <AddProject
          tracked={new Set(state.projects.map((t) => t.project.root))}
          onClose={() => setCreating(false)}
          onMessage={setToast}
        />
      )}
      <div className="cards" onScroll={() => setHover(null)}>
        {state.projects.length === 0 && other.length === 0 && (
          <p className="empty">No projects yet. Start a session, or use + above.</p>
        )}
        {state.projects
          .filter((t) => !t.shelved)
          .map((t) => (
            <ProjectSection
              key={t.project.root}
              project={t.project}
              sessions={byRoot.get(t.project.root) ?? []}
              hasTasks={state.task_projects.includes(t.project.root)}
              sessionOrder={t.session_order}
              {...sectionProps}
            />
          ))}
        {other.length > 0 && (
          <ProjectSection
            project={null}
            sessions={other}
            hasTasks={false}
            sessionOrder={[]}
            {...sectionProps}
          />
        )}
      </div>
      {hover && hovered && !menu && hover.el.isConnected && (
        <HoverCard anchor={hover.el}>
          <SessionCard session={hovered} {...cards} expanded />
        </HoverCard>
      )}
      {menu && menuSession && (
        <SessionMenu
          session={menuSession}
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          onMessage={setToast}
          onRename={(s, name) =>
            invoke("rename_session", { sessionId: s.session_id, name }).catch(report)
          }
          onEnd={(s) =>
            invoke("session_action", { sessionId: s.session_id, action: "end" }).catch(report)
          }
          onRemove={(s) => invoke("dismiss_session", { sessionId: s.session_id }).catch(report)}
          onAction={(s, action) =>
            invoke("session_action", { sessionId: s.session_id, action }).catch(report)
          }
        />
      )}
      <Drawer
        shelved={state.projects.filter((t) => t.shelved)}
        byRoot={byRoot}
        fresh={fresh}
        dragging={draggingProject}
        onShelve={(root) => shelve(root, true)}
        onUnshelve={(root) => shelve(root, false)}
        onDropAt={(dragged, onto, place) => placeProject(dragged, onto, place, true)}
      />
      <UsageFooter usage={state.usage} now={now} />
      {toast && <div className="toast">{toast}</div>}
    </main>
  );
}

export default App;
