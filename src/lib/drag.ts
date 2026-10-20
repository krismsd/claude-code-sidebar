// The project header or drawer tile being dragged. dataTransfer can't be read during
// dragover, so drop targets look here to tell, e.g., a project dragged over itself.
export const DRAG_TYPE = "application/x-cc-project";

let current: string | null = null;

export function startProjectDrag(e: React.DragEvent, root: string) {
  current = root;
  e.dataTransfer.setData(DRAG_TYPE, root);
  e.dataTransfer.effectAllowed = "move";
}

export function draggedProject(): string | null {
  return current;
}

export function endProjectDrag() {
  current = null;
}

// Session cards are dragged to reorder them within their project.
export const SESSION_DRAG_TYPE = "application/x-cc-session";

let session: { root: string; id: string } | null = null;

export function startSessionDrag(e: React.DragEvent, root: string, id: string) {
  session = { root, id };
  e.dataTransfer.setData(SESSION_DRAG_TYPE, id);
  e.dataTransfer.effectAllowed = "move";
}

export function draggedSession(): { root: string; id: string } | null {
  return session;
}

export function endSessionDrag() {
  session = null;
}

/** Which half of `el` the pointer is in: "before" (top/left) or "after". */
export function dropPlace(e: React.DragEvent, horizontal = false): "before" | "after" {
  const r = e.currentTarget.getBoundingClientRect();
  return horizontal
    ? e.clientX < r.left + r.width / 2
      ? "before"
      : "after"
    : e.clientY < r.top + r.height / 2
      ? "before"
      : "after";
}
