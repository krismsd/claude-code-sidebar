import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { placeMenu } from "../lib/menu";

interface Props {
  /** What opened it: a button (the menu drops from it) or a right-click point. */
  anchor: HTMLElement | { x: number; y: number };
  onClose: () => void;
  className?: string;
  children: ReactNode;
}

/** What ↑/↓ move between, and what gets focus first. */
const ITEMS = "button:not(:disabled), input";

/**
 * A pop-up menu: drawn above everything (a portal, so no scrolling box clips it), measured and
 * placed inside the window (flipping up or left, scrolling inside only when the window is too
 * short), and modal while open:
 * - a click outside closes it and does nothing else (a right-click still opens its own menu);
 * - the wheel and scroll keys don't scroll what's behind it, so it stays by its anchor;
 * - the keyboard is its own: ↑/↓/Home/End move between items, Enter picks, Esc and Tab-away
 *   close, and nothing reaches the card shortcuts behind it (Enter, A/D, arrows);
 * - focus goes to its first item, and back to where it was when it closes.
 */
export function Menu({ anchor, onClose, className = "", children }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  useLayoutEffect(() => {
    close.current = onClose;
  });

  // Measure and place on every render: live updates can move the anchor or grow the menu.
  // Written straight to the element (no state): it's shown only once placed, so it never
  // flashes in the wrong spot.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (anchor instanceof HTMLElement && !anchor.isConnected) return close.current();
    // Measure at natural height: drop the cap from the last placement first.
    el.style.maxHeight = "";
    const { width, height } = el.getBoundingClientRect();
    const at = anchor instanceof HTMLElement ? anchor.getBoundingClientRect() : anchor;
    const p = placeMenu(at, width, height, window.innerWidth, window.innerHeight);
    el.style.left = `${p.left}px`;
    el.style.top = `${p.top}px`;
    el.style.maxHeight = p.maxHeight === undefined ? "" : `${p.maxHeight}px`;
    el.style.visibility = "visible";
  });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const back = document.activeElement as HTMLElement | null;
    const items = () => [...el.querySelectorAll<HTMLElement>(ITEMS)];
    items()[0]?.focus({ preventScroll: true });

    const inside = (t: EventTarget | null) => t instanceof Node && el.contains(t);
    // The click that dismisses the menu is eaten: it mustn't press a card's Allow button or
    // open a session on the way.
    const eatClick = (e: MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();
    };
    const onDown = (e: MouseEvent) => {
      if (inside(e.target)) return;
      close.current();
      if (e.button !== 0) return; // right-click: let the next context menu open
      e.stopPropagation();
      e.preventDefault();
      window.addEventListener("click", eatClick, { capture: true, once: true });
      // A drag-off or a click that never comes mustn't eat a later one.
      setTimeout(() => window.removeEventListener("click", eatClick, { capture: true }), 500);
    };
    const onWheel = (e: WheelEvent) => {
      if (!inside(e.target)) e.preventDefault();
    };
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLInputElement;
      const list = items();
      const at = list.indexOf(document.activeElement as HTMLElement);
      const move = (i: number) => list[(i + list.length) % list.length]?.focus();
      if (e.key === "Escape") close.current();
      else if (e.key === "Tab") close.current();
      else if (e.key === "ArrowDown" && !typing) move(at + 1);
      else if (e.key === "ArrowUp" && !typing) move(at < 0 ? -1 : at - 1);
      else if (e.key === "Home" && !typing) move(0);
      else if (e.key === "End" && !typing) move(-1);
      else if (inside(e.target)) {
        // Enter/Space on an item, typing in the rename box: theirs. The card shortcuts on
        // the window check for an open menu and stay out of it.
        return;
      }
      e.preventDefault();
      e.stopPropagation();
    };
    const onBlur = () => close.current();
    const onResize = () => close.current();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("wheel", onWheel, { capture: true });
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onResize);
      // Focus goes back only if nothing else took it (a picked item may open a panel that
      // focuses its own input).
      const lost = !document.activeElement || document.activeElement === document.body;
      if (lost && back?.isConnected) back.focus({ preventScroll: true });
    };
  }, []);

  return createPortal(
    <div
      className={`menu ${className}`}
      ref={ref}
      role="menu"
      style={{ left: 0, top: 0, visibility: "hidden" }}
    >
      {children}
    </div>,
    document.body,
  );
}

/** True while a pop-up menu is open: window-wide shortcuts leave the keyboard to it. */
export const menuOpen = () => document.querySelector(".menu") !== null;
