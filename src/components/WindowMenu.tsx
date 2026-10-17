import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Monitor } from "../lib/types";

interface Props {
  floating: boolean;
  onClose: () => void;
  onError: (e: unknown) => void;
}

/** ⚙ menu: docked/floating, which monitor, settings files. */
export function WindowMenu({ floating, onClose, onError }: Props) {
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
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const set = (args: { mode?: string; monitor?: number }) => {
    invoke("set_window", args).catch(onError);
    onClose();
  };

  return (
    <div className="window-menu" ref={ref}>
      <div className="menu-label">Window</div>
      <button className={floating ? "" : "checked"} onClick={() => set({ mode: "docked" })}>
        Docked (reserves its space)
      </button>
      <button className={floating ? "checked" : ""} onClick={() => set({ mode: "floating" })}>
        Floating window
      </button>
      {monitors.length > 0 && <div className="menu-label">Monitor</div>}
      {monitors.map((m) => (
        <button key={m.number} onClick={() => set({ monitor: m.number })}>
          {m.number} · {m.width}×{m.height} · {m.scale_percent}%{m.primary ? " · primary" : ""}
        </button>
      ))}
      {monitors.length > 1 && (
        <button onClick={() => set({ monitor: 0 })}>Primary (follows it)</button>
      )}
      <div className="menu-label">Settings</div>
      <button onClick={() => (invoke("open_settings_file").catch(onError), onClose())}>
        config.toml…
      </button>
      <button onClick={() => (invoke("open_projects_file").catch(onError), onClose())}>
        projects.toml…
      </button>
    </div>
  );
}
