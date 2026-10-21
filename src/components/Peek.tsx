import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UiState } from "../lib/types";
import { SessionCard } from "./SessionCard";
import "../App.css";

const noop = () => {};

/** The peek window: one session's expanded card, shown beside the collapsed strip. It's never
 * focused or hovered; it just reports its height so the window fits the card. */
export function Peek() {
  const [state, setState] = useState<UiState | null>(null);
  const [id, setId] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<UiState>("get_state").then(setState);
    const a = listen<UiState>("state", (e) => setState(e.payload));
    const b = listen<string>("peek", (e) => {
      setNow(Date.now());
      setId(e.payload);
    });
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      a.then((f) => f());
      b.then((f) => f());
      clearInterval(t);
    };
  }, []);

  const session = state?.sessions.find((s) => s.session_id === id);

  useLayoutEffect(() => {
    if (ref.current && session) invoke("peek_resize", { height: ref.current.offsetHeight });
  });

  return (
    <div ref={ref} className="peek">
      {session && (
        <SessionCard
          session={session}
          now={now}
          away={state?.away ?? []}
          fresh={new Set()}
          onActivate={noop}
          onDismissAlert={noop}
          onAnswer={noop}
          onHover={noop}
          onMenu={noop}
          expanded
        />
      )}
    </div>
  );
}
