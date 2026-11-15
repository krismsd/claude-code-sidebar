// Where a pop-up menu goes, from the measured menu and the window: pure, unit-tested.

export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Placement {
  left: number;
  top: number;
  /** Set when neither side has room for the whole menu: it scrolls inside this height. */
  maxHeight?: number;
}

/** Gap kept between a menu and the window edge. */
export const EDGE = 4;

/**
 * Place a `w`×`h` menu in a `vw`×`vh` window, never past its edges.
 *
 * `anchor` is what opened it: a button (the menu drops below it, right edges aligned, or
 * opens above when there's no room below) or a point (a right-click: the menu's corner is at
 * the pointer, flipped left or up when it would run off). When neither side has room, it takes
 * the roomier side and scrolls.
 */
export function placeMenu(
  anchor: Rect | { x: number; y: number },
  w: number,
  h: number,
  vw: number,
  vh: number,
): Placement {
  // A DOMRect has x and y too: tell them apart by `right`.
  const isPoint = !("right" in anchor);
  const a: Rect = isPoint
    ? { left: anchor.x, right: anchor.x, top: anchor.y, bottom: anchor.y }
    : anchor;
  const gap = isPoint ? 0 : 2;

  // Horizontal: a point opens rightwards, a button aligns to its right edge; then keep inside.
  let left = isPoint ? (a.left + w <= vw - EDGE ? a.left : a.left - w) : a.right - w;
  left = Math.max(EDGE, Math.min(left, vw - EDGE - w));

  // Vertical: below, else above, else the roomier side with a scroll.
  const below = vh - EDGE - (a.bottom + gap);
  const above = a.top - gap - EDGE;
  if (h <= below) return { left, top: a.bottom + gap };
  if (h <= above) return { left, top: a.top - gap - h };
  if (below >= above) return { left, top: a.bottom + gap, maxHeight: Math.max(below, 0) };
  return { left, top: EDGE, maxHeight: above };
}
