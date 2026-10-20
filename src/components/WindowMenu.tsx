import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Monitor } from "../lib/types";

interface Props {
  floating: boolean;
  /** Configured monitor: 0 = primary, n = nth from the left. */
  monitor: number;
  onClose: () => void;
  onError: (e: unknown) => void;
}

/** ⚙ menu: where the sidebar sits (docked on a monitor, or floating) and the settings window. */
export function WindowMenu({ floating, monitor, onClose, onError }: Props) {
  const [monitors, setMonitors] = useState<Monitor[]>([]);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    invoke<Monitor[]>("list_monitors").then(setMonitors).catch(onError);
  }, [onError]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // Clicking anywhere outside the sidebar (another window) closes it too.
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  const set = (args: { mode?: string; monitor?: number }) => {
    invoke("set_window", args).catch(onError);
    onClose();
  };

  return (
    <div className="window-menu" ref={ref}>
      <div className="menu-label">Position</div>
      {monitors.map((m) => {
        const current = !floating && (monitor === m.number || (monitor === 0 && m.primary));
        return (
          <button
            key={m.number}
            className={current ? "checked" : ""}
            onClick={() => set({ mode: "docked", monitor: m.number })}
          >
            Docked on {monitors.length > 1 ? `monitor ${m.number}` : "this monitor"}
            <span className="menu-note">
              {m.width}×{m.height} · {m.scale_percent}%{m.primary ? " · primary" : ""}
            </span>
          </button>
        );
      })}
      <button className={floating ? "checked" : ""} onClick={() => set({ mode: "floating" })}>
        Floating window
        <span className="menu-note">movable, reserves no space</span>
      </button>
      <div className="menu-label">Settings</div>
      <button onClick={() => (invoke("open_settings").catch(onError), onClose())}>Settings…</button>
    </div>
  );
}
