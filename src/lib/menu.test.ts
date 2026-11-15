import { describe, expect, it } from "vitest";
import { EDGE, placeMenu } from "./menu";

// A 320×900 sidebar; a 170×190 menu (the project ⋯ menu).
const VW = 320;
const VH = 900;
const button = (top: number, right = 310) => ({ left: right - 20, right, top, bottom: top + 20 });

describe("placeMenu", () => {
  it("drops below a button, right edges aligned", () => {
    expect(placeMenu(button(100), 170, 190, VW, VH)).toEqual({ left: 140, top: 122 });
  });

  it("treats a DOMRect (which also has x and y) as a button, not a point", () => {
    const rect = { ...button(100), x: 290, y: 100, width: 20, height: 20 };
    expect(placeMenu(rect, 170, 190, VW, VH)).toEqual({ left: 140, top: 122 });
  });

  it("opens above a button near the bottom (the last project)", () => {
    const p = placeMenu(button(800), 170, 190, VW, VH);
    expect(p).toEqual({ left: 140, top: 800 - 2 - 190 });
  });

  it("never runs past the window: too tall for either side, it scrolls on the roomier one", () => {
    const below = placeMenu(button(200), 170, 1000, VW, VH);
    expect(below).toEqual({ left: 140, top: 222, maxHeight: VH - EDGE - 222 });
    const above = placeMenu(button(700), 170, 1000, VW, VH);
    expect(above).toEqual({ left: 140, top: EDGE, maxHeight: 700 - 2 - EDGE });
  });

  it("keeps a wide menu inside the window horizontally", () => {
    // "N closed" sits near the left; its 260 px menu right-aligned would go off the left edge.
    expect(placeMenu(button(100, 60), 260, 100, VW, VH).left).toBe(EDGE);
    expect(placeMenu(button(100), 400, 100, VW, VH).left).toBe(EDGE);
  });

  it("opens a right-click menu at the pointer, flipping left and up near the edges", () => {
    expect(placeMenu({ x: 50, y: 100 }, 200, 220, VW, VH)).toEqual({ left: 50, top: 100 });
    expect(placeMenu({ x: 250, y: 850 }, 200, 220, VW, VH)).toEqual({ left: 50, top: 630 });
  });
});
