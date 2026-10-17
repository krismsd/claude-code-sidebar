import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Task } from "../lib/types";

const CACHE_MS = 5 * 60_000;
const cache = new Map<string, { tasks: Task[]; at: number }>();

interface Props {
  root: string;
  onClose: () => void;
  onMessage: (msg: string) => void;
}

/** A project's tasks from its issue tracker, via Claude + a connector (~10 s to load). */
export function TasksPanel({ root, onClose, onMessage }: Props) {
  const [tasks, setTasks] = useState<Task[] | null>(() => {
    const hit = cache.get(root);
    return hit && Date.now() - hit.at < CACHE_MS ? hit.tasks : null;
  });
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    const hit = cache.get(root);
    if (reload === 0 && hit && Date.now() - hit.at < CACHE_MS) return;
    let live = true;
    invoke<Task[]>("list_tasks", { root })
      .then((t) => {
        cache.set(root, { tasks: t, at: Date.now() });
        if (live) {
          setTasks(t);
          setError(null);
        }
      })
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [root, reload]);

  const start = async (task: Task) => {
    try {
      await invoke("start_task", { root, task });
      onMessage(`Starting a session for ${task.id}…`);
      onClose();
    } catch (e) {
      onMessage(String(e));
    }
  };

  const q = filter.trim().toLowerCase();
  const shown = (tasks ?? []).filter(
    (t) => !q || `${t.id} ${t.title} ${t.status}`.toLowerCase().includes(q),
  );

  return (
    <div className="tasks">
      <div className="tasks-head">
        <span>Tasks</span>
        <span>
          <button
            className="x"
            title="Refresh"
            onClick={() => {
              setTasks(null);
              setError(null);
              setReload((n) => n + 1);
            }}
          >
            ↻
          </button>
          <button className="x" title="Close" onClick={onClose}>
            ✕
          </button>
        </span>
      </div>
      {tasks === null && !error && (
        <p className="muted small">Asking Claude for tasks… (about 10 seconds)</p>
      )}
      {error && <p className="task-error small">{error}</p>}
      {tasks !== null && tasks.length > 6 && (
        <input
          className="task-filter"
          placeholder="Filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
        />
      )}
      {tasks?.length === 0 && <p className="muted small">No tasks found.</p>}
      <ul className="task-list">
        {shown.map((t) => (
          <li key={t.id}>
            <button
              onClick={() => start(t)}
              title={`${t.summary}\n\nClick to start a session on it`}
            >
              <span className="task-id">{t.id}</span>
              <span className="task-title">{t.title}</span>
              {t.status && <span className="task-status">{t.status}</span>}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
