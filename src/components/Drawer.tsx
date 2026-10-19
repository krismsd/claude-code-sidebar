import { type CSSProperties } from "react";
import type { Session, Tracked } from "../lib/types";
import { projectStatus } from "../lib/view";

interface Props {
  shelved: Tracked[];
  byRoot: Map<string, Session[]>;
  fresh: Set<string>;
  /** A project header is being dragged: show the drop zone even when the drawer is empty. */
  dragging: boolean;
  onShelve: (root: string) => void;
  onUnshelve: (root: string) => void;
}

const DRAG_TYPE = "application/x-cc-project";

/** "MyApp" → "MA", "cc-sidebar" → "CS", "api" → "AP". */
function initials(name: string): string {
  const words = name.split(/[\s\-_.]+/).filter(Boolean);
  const caps = name.match(/[A-Z]/g) ?? [];
  if (words.length > 1) return (words[0][0] + words[1][0]).toUpperCase();
  if (caps.length > 1) return caps[0] + caps[1];
  return name.slice(0, 2).toUpperCase();
}

/** The drawer (SPEC §3.12): projects put away as small tiles at the bottom of the sidebar.
 * Drop a project header here (or use its ⋯ menu) to put it away; click a tile, or drag it
 * back into the list, to take it out. A tile's dot still shows its sessions' state. */
export function Drawer({ shelved, byRoot, fresh, dragging, onShelve, onUnshelve }: Props) {
  if (shelved.length === 0 && !dragging) return null;
  return (
    <div
      // The drop-zone look is only for an empty drawer: restyling it mid-drag would move the
      // tile being dragged out from under the cursor.
      className={`drawer${dragging && shelved.length === 0 ? " dragging" : ""}`}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes(DRAG_TYPE)) e.preventDefault();
      }}
      onDrop={(e) => {
        const root = e.dataTransfer.getData(DRAG_TYPE);
        if (root) onShelve(root);
      }}
    >
      {shelved.length === 0 && <span className="muted small">Drop here to put it away</span>}
      {shelved.map(({ project: p }) => {
        const sessions = byRoot.get(p.root) ?? [];
        const status = projectStatus(sessions, fresh);
        return (
          // A div, not a button: WebView2 won't start a drag from a button.
          <div
            key={p.root}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onUnshelve(p.root)}
            className={`drawer-tile${status.flashing ? " flash" : ""}`}
            style={{ "--project": p.color } as CSSProperties}
            title={`${p.name}${status.open ? ` · ${status.open} open` : ""}\nClick or drag back to take it out`}
            draggable
            onDragStart={(e) => e.dataTransfer.setData(DRAG_TYPE, p.root)}
            onClick={() => onUnshelve(p.root)}
          >
            <span className="drawer-mark">{p.emoji ?? initials(p.name)}</span>
            <span className="drawer-name">{p.name}</span>
            {status.state && <span className={`dot ${status.state}`} />}
          </div>
        );
      })}
    </div>
  );
}
