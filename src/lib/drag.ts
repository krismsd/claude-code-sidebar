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
