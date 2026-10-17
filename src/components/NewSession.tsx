import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Recent } from "../lib/types";

interface Props {
  onClose: () => void;
  onMessage: (msg: string) => void;
}

/** "+" panel (SPEC §3.11): recent projects, or any WSL folder. */
export function NewSession({ onClose, onMessage }: Props) {
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

  const start = async (p: string) => {
    try {
      await invoke("new_session", { path: p });
      onMessage("Starting a new session…");
      onClose();
    } catch (e) {
      onMessage(String(e));
    }
  };

  const browse = async () => {
    const picked = await invoke<string | null>("pick_folder");
    if (picked) setPath(picked);
  };

  return (
    <div className="new-session">
      <div className="new-session-head">
        <span>New session</span>
        <button className="x" onClick={onClose} title="Close">
          ✕
        </button>
      </div>
      {recents === null && <p className="muted small">Loading recent projects…</p>}
      {recents?.length === 0 && <p className="muted small">No recent projects.</p>}
      <ul className="recents">
        {recents?.map((r) => (
          <li key={r.path}>
            <button onClick={() => start(r.path)} title={r.path}>
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
          if (path.trim()) start(path.trim());
        }}
      >
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Other: /home/you/dev/project"
          spellCheck={false}
        />
        <button type="button" onClick={browse} title="Browse…">
          …
        </button>
        <button type="submit" disabled={!path.trim()}>
          Start
        </button>
      </form>
    </div>
  );
}
