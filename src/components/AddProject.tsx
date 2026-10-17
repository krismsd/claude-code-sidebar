import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Recent } from "../lib/types";

interface Props {
  /** Roots already in the sidebar: left out of the recents. */
  tracked: Set<string>;
  onClose: () => void;
  onMessage: (msg: string) => void;
}

/** "+" panel: add a project to the sidebar, from recent ones or any WSL folder. */
export function AddProject({ tracked, onClose, onMessage }: Props) {
  const [recents, setRecents] = useState<Recent[] | null>(null);
  const [path, setPath] = useState("");

  useEffect(() => {
    invoke<Recent[]>("recent_projects")
      .then(setRecents)
      .catch((e) => {
        setRecents([]);
        onMessage(String(e));
      });
  }, [onMessage]);

  const add = async (p: string) => {
    try {
      const name = await invoke<string>("track_project", { path: p });
      onMessage(`Added ${name}. Use its + to start a session.`);
      onClose();
    } catch (e) {
      onMessage(String(e));
    }
  };
  const fresh = recents?.filter((r) => !tracked.has(r.path)) ?? null;

  const browse = async () => {
    const picked = await invoke<string | null>("pick_folder");
    if (picked) setPath(picked);
  };

  return (
    <div className="new-session">
      <div className="new-session-head">
        <span>Add project</span>
        <button className="x" onClick={onClose} title="Close">
          ✕
        </button>
      </div>
      {fresh === null && <p className="muted small">Loading recent projects…</p>}
      {fresh?.length === 0 && (
        <p className="muted small">No other recent projects. Pick a folder below.</p>
      )}
      <ul className="recents">
        {fresh?.map((r) => (
          <li key={r.path}>
            <button onClick={() => add(r.path)} title={r.path}>
              <span className="dot" style={{ background: r.color }} />
              <span className="recent-name">{r.name}</span>
              <span className="muted small recent-path">{r.path}</span>
            </button>
          </li>
        ))}
      </ul>
      <form
        className="other"
        onSubmit={(e) => {
          e.preventDefault();
          if (path.trim()) add(path.trim());
        }}
      >
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Folder: /home/you/dev/project"
          spellCheck={false}
        />
        <button type="button" onClick={browse} title="Browse…">
          …
        </button>
        <button type="submit" disabled={!path.trim()}>
          Add
        </button>
      </form>
    </div>
  );
}
