import { useEffect, useState, type RefObject } from "react";
import { offscreen, type Span } from "../lib/offscreen";

interface Hidden {
  above: Span[];
  below: Span[];
}

const same = (a: Span[], b: Span[]) => a.length === b.length && a.every((x, i) => x.id === b[i].id);

/** "↑ 1 ready" / "↓ 2 ready" pills on the list's edges while unread cards are scrolled out of
 * view that way (SPEC §3.17). A click scrolls the nearest one in and pulses its ring. Nothing
 * else moves: the list keeps its order and scroll position. */
export function OffscreenPills({ scroller }: { scroller: RefObject<HTMLDivElement | null> }) {
  const [hidden, setHidden] = useState<Hidden>({ above: [], below: [] });

  // Measured on scroll, on resize (the window, or the list growing: a to-do panel opening) and
  // when the cards change (one turns unread or is looked at, folds, comes or goes).
  useEffect(() => {
    const box = scroller.current;
    if (!box) return;
    const measure = () => {
      const view = box.getBoundingClientRect();
      const cards = [...box.querySelectorAll<HTMLElement>(".card.unread[data-session]")].map(
        (el) => {
          const r = el.getBoundingClientRect();
          return { id: el.dataset.session!, top: r.top, bottom: r.bottom };
        },
      );
      const next = offscreen(cards, view);
      setHidden((prev) =>
        same(prev.above, next.above) && same(prev.below, next.below) ? prev : next,
      );
    };
    box.addEventListener("scroll", measure, { passive: true });
    const resize = new ResizeObserver(measure);
    resize.observe(box);
    if (box.firstElementChild) resize.observe(box.firstElementChild);
    const changes = new MutationObserver(measure);
    changes.observe(box, { subtree: true, childList: true, attributeFilter: ["class"] });
    return () => {
      box.removeEventListener("scroll", measure);
      resize.disconnect();
      changes.disconnect();
    };
  }, [scroller]);

  const reveal = (span: Span) => {
    const el = scroller.current?.querySelector<HTMLElement>(
      `.card[data-session="${CSS.escape(span.id)}"]`,
    );
    if (!el) return;
    el.scrollIntoView({ block: "nearest", behavior: "smooth" });
    // Restart the pulse if it's still running from a previous click.
    el.classList.remove("unread-pulse");
    void el.offsetWidth;
    el.classList.add("unread-pulse");
    el.addEventListener("animationend", () => el.classList.remove("unread-pulse"), { once: true });
  };

  const names = (spans: Span[]) =>
    spans
      .map(
        (s) =>
          scroller.current?.querySelector(`.card[data-session="${CSS.escape(s.id)}"] .card-name`)
            ?.textContent ?? "",
      )
      .filter(Boolean)
      .join("\n");

  const pill = (where: "above" | "below") => {
    const spans = hidden[where];
    if (spans.length === 0) return null;
    return (
      <button
        className={`offscreen-pill ${where}`}
        title={`Ready, not looked at yet:\n${names(spans)}`}
        onClick={() => reveal(spans[0])}
      >
        <span className="unread-dot" />
        {where === "above" ? "↑" : "↓"} {spans.length} ready
      </button>
    );
  };

  return (
    <>
      {pill("above")}
      {pill("below")}
    </>
  );
}
