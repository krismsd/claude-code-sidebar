// Which cards are scrolled out of view, above or below the list's visible area (SPEC §3.17).

export interface Span {
  id: string;
  top: number;
  bottom: number;
}

/** How much of a card must show for it to count as seen: about its title bar. */
export const SEEN = 24;

/** The cards out of view above and below `view` (both in the same coordinates, e.g. client
 * pixels), nearest first. A card counts as in view once its title bar's worth shows. */
export function offscreen(
  cards: Span[],
  view: { top: number; bottom: number },
): { above: Span[]; below: Span[] } {
  const above = cards.filter((c) => c.bottom < view.top + SEEN).sort((a, b) => b.bottom - a.bottom);
  const below = cards.filter((c) => c.top > view.bottom - SEEN).sort((a, b) => a.top - b.top);
  return { above, below };
}
