import { useState, type CSSProperties } from "react";
import type { Session, Tracked } from "../lib/types";
import { projectStatus } from "../lib/view";
import {
  DRAG_TYPE,
  draggedProject,
  dropPlace,
  endProjectDrag,
  startProjectDrag,
} from "../lib/drag";

interface Props {
  shelved: Tracked[];
  byRoot: Map<string, Session[]>;
  fresh: Set<string>;
  /** A project header is being dragged: show the drop zone even when the drawer is empty. */
  dragging: boolean;
  onShelve: (root: string) => void;
  onUnshelve: (root: string) => void;
  /** A project dropped just before or after a tile: put it away there. */
  onDropAt: (dragged: string, onto: string, place: "before" | "after") => void;
}

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
export function Drawer({
  shelved,
  byRoot,
  fresh,
  dragging,
  onShelve,
  onUnshelve,
  onDropAt,
}: Props) {
  const [dropAt, setDropAt] = useState<{ root: string; place: "before" | "after" } | null>(null);
  if (shelved.length === 0 && !dragging) return null;
  return (
    <div
      // The drop-zone look is only for an empty drawer: restyling it mid-drag would move the
      // tile being dragged out from under the cursor.
      className={`drawer${dragging && shelved.length === 0 ? " dragging" : ""}`}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
      }}
      onDrop={(e) => {
        const root = e.dataTransfer.getData(DRAG_TYPE);
        setDropAt(null);
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
            className={`drawer-tile${status.flashing ? " flash" : ""}${dropAt?.root === p.root ? ` drop-${dropAt.place}` : ""}`}
            style={{ "--project": p.color } as CSSProperties}
            title={`${p.name}${status.open ? ` · ${status.open} open` : ""}\nClick or drag back to take it out`}
            draggable
            onDragStart={(e) => startProjectDrag(e, p.root)}
            onDragEnd={() => (endProjectDrag(), setDropAt(null))}
            // Drop on a tile's left or right half to put a project just there.
            onDragOver={(e) => {
              if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              const dragged = draggedProject();
              setDropAt(dragged === p.root ? null : { root: p.root, place: dropPlace(e, true) });
            }}
            onDragLeave={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node)) setDropAt(null);
            }}
            onDrop={(e) => {
              const dragged = e.dataTransfer.getData(DRAG_TYPE);
              const place = dropAt?.place;
              setDropAt(null);
              e.stopPropagation();
              if (!dragged || dragged === p.root) return;
              if (place) onDropAt(dragged, p.root, place);
              else onShelve(dragged);
            }}
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
