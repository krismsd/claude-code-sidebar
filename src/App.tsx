import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Session, UiState } from "./lib/types";
import { ProjectSection, type CardHandlers } from "./components/ProjectSection";
import { SessionCard } from "./components/SessionCard";
import { SessionMenu } from "./components/SessionMenu";
import { AddProject } from "./components/AddProject";
import { Strip } from "./components/Strip";
import { UsageFooter } from "./components/UsageFooter";
import { WindowMenu } from "./components/WindowMenu";
import { freshKeys, groupSessions } from "./lib/view";
import "./App.css";

type Outcome = "focused" | "resumed" | "tab_not_found" | "untagged" | "failed";

const OUTCOME_MESSAGE: Partial<Record<Outcome, string>> = {
  resumed: "Resuming in a new tab…",
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
 * edge hovers the next card. Shifted up only if it would run off the bottom of the window. */
function HoverCard({ rect, children }: { rect: DOMRect; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const max = window.innerHeight - el.offsetHeight - 6;
    el.style.top = `${Math.max(6, Math.min(rect.top, max))}px`;
  });
  return (
    <div
      ref={ref}
      className="hover-card"
      style={{ top: rect.top, left: rect.left, width: rect.width }}
    >
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
    const t = setTimeout(
      () => setFresh((f) => new Set([...f].filter((k) => !added.includes(k)))),
      FLASH_MS,
    );
    return () => clearTimeout(t);
  }, [sessions]);
  return fresh;
}

function App() {
  const [state, setState] = useState<UiState | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [windowMenu, setWindowMenu] = useState(false);
  const [hover, setHover] = useState<{ id: string; rect: DOMRect } | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const openTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
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
    if (!el) {
      closeTimer.current = setTimeout(() => setHover(null), HOVER_CLOSE_MS);
      return;
    }
    const next = { id: s.session_id, rect: el.getBoundingClientRect() };
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
        onExpand={() => setCollapsed(false)}
      />
    );
  const drag = state.floating ? { "data-tauri-drag-region": true } : {};

  const { byRoot, other } = groupSessions(state);
  const openCount = state.sessions.filter((s) => s.status !== "closed").length;
  const hovered = hover && state.sessions.find((s) => s.session_id === hover.id);
  const menuSession = menu && state.sessions.find((s) => s.session_id === menu.id);

  const sectionProps = {
    cards,
    onNewSession: (root: string) =>
      invoke("new_session", { path: root })
        .then(() => setToast("Starting a new session…"))
        .catch(report),
    onProjectAction: (root: string, action: "folder" | "vscode") =>
      invoke("project_action", { root, action }).catch(report),
    onCloseProject: (root: string) => invoke("close_project", { root }).catch(report),
    onSetupTasks: () => invoke("open_projects_file").catch(report),
    onMessage: setToast,
    onDropProject: (dragged: string, onto: string) =>
      invoke("move_project", {
        root: dragged,
        to: state.projects.findIndex((t) => t.project.root === onto),
      }).catch(report),
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
        {state.projects.map((t) => (
          <ProjectSection
            key={t.project.root}
            project={t.project}
            sessions={byRoot.get(t.project.root) ?? []}
            hasTasks={state.task_projects.includes(t.project.root)}
            {...sectionProps}
          />
        ))}
        {other.length > 0 && (
          <ProjectSection project={null} sessions={other} hasTasks={false} {...sectionProps} />
        )}
      </div>
      {hover && hovered && !menu && (
        <HoverCard rect={hover.rect}>
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
        />
      )}
      <UsageFooter usage={state.usage} now={now} />
      {toast && <div className="toast">{toast}</div>}
    </main>
  );
}

export default App;
