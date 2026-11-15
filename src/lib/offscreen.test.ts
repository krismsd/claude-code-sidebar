import { describe, expect, it } from "vitest";
import { offscreen, SEEN } from "./offscreen";

const view = { top: 100, bottom: 500 };
const card = (id: string, top: number, h = 110) => ({ id, top, bottom: top + h });

describe("offscreen", () => {
  it("splits cards above and below the view, nearest first", () => {
    const cards = [
      card("far-up", -300),
      card("up", -50),
      card("in", 200),
      card("down", 600),
      card("far-down", 900),
    ];
    const { above, below } = offscreen(cards, view);
    expect(above.map((c) => c.id)).toEqual(["up", "far-up"]);
    expect(below.map((c) => c.id)).toEqual(["down", "far-down"]);
  });

  it("counts a card as seen once its title bar shows", () => {
    // Just its title bar's worth showing at the bottom: seen. A sliver less: not.
    expect(offscreen([card("a", view.bottom - SEEN)], view).below).toHaveLength(0);
    expect(offscreen([card("a", view.bottom - SEEN + 1)], view).below).toHaveLength(1);
    // The same at the top, by its bottom edge.
    expect(offscreen([card("a", view.top + SEEN - 110)], view).above).toHaveLength(0);
    expect(offscreen([card("a", view.top + SEEN - 111)], view).above).toHaveLength(1);
  });

  it("is empty when everything is in view", () => {
    expect(offscreen([card("a", 120), card("b", 300)], view)).toEqual({ above: [], below: [] });
  });
});
