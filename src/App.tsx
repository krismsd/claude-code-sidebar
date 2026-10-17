import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Session, SessionAction, UiState } from "./lib/types";
import { ProjectSection, type CardHandlers } from "./components/ProjectSection";
import { Flyout } from "./components/Flyout";
import { AddProject } from "./components/AddProject";
import { Strip } from "./components/Strip";
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
const FLYOUT_OPEN_MS = 300;
const FLYOUT_CLOSE_MS = 200;

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
  const [flyout, setFlyout] = useState<{ id: string; rect: DOMRect } | null>(null);
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

  // Flyout: opens after a short hover; while one is open, moving to another card switches it
  // straight away; leaving gives a moment to move onto the flyout itself.
  const keepFlyout = () => clearTimeout(closeTimer.current);
  const dropFlyout = () => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setFlyout(null), FLYOUT_CLOSE_MS);
  };
  const onHover = (s: Session, el: HTMLElement | null) => {
    clearTimeout(openTimer.current);
    if (!el) return dropFlyout();
    keepFlyout();
    const next = { id: s.session_id, rect: el.getBoundingClientRect() };
    if (flyout) setFlyout(next);
    else openTimer.current = setTimeout(() => setFlyout(next), FLYOUT_OPEN_MS);
  };

  const cards: CardHandlers = {
    now,
    fresh,
    onHover,
    onActivate: async (s) => {
      setFlyout(null);
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
    setFlyout(null);
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
        onExpand={() => setCollapsed(false)}
      />
    );
  const drag = state.floating ? { "data-tauri-drag-region": true } : {};

  const { byRoot, other } = groupSessions(state);
  const openCount = state.sessions.filter((s) => s.status !== "closed").length;
  const flyoutSession = flyout && state.sessions.find((s) => s.session_id === flyout.id);

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
      <div className="cards" onScroll={() => setFlyout(null)}>
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
      {flyout && flyoutSession && (
        <Flyout
          session={flyoutSession}
          anchor={flyout.rect}
          onEnter={keepFlyout}
          onLeave={dropFlyout}
          onAction={(s: Session, action: SessionAction) =>
            invoke("session_action", { sessionId: s.session_id, action }).catch(report)
          }
          onDismiss={(s: Session) => {
            setFlyout(null);
            invoke("dismiss_session", { sessionId: s.session_id }).catch(report);
          }}
        />
      )}
      {toast && <div className="toast">{toast}</div>}
    </main>
  );
}

export default App;
