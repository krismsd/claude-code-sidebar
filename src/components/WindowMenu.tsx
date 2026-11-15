import { useEffect, useState } from "react";
import { Menu } from "./Menu";
import { invoke } from "@tauri-apps/api/core";
import type { Monitor } from "../lib/types";

interface Props {
  /** The ⚙ button it opens from. */
  anchor: HTMLElement;
  floating: boolean;
  /** Configured monitor: 0 = primary, n = nth from the left. */
  monitor: number;
  onClose: () => void;
  onError: (e: unknown) => void;
}

/** ⚙ menu: where the sidebar sits (docked on a monitor, or floating) and the settings window. */
export function WindowMenu({ anchor, floating, monitor, onClose, onError }: Props) {
  const [monitors, setMonitors] = useState<Monitor[]>([]);

  useEffect(() => {
    invoke<Monitor[]>("list_monitors").then(setMonitors).catch(onError);
  }, [onError]);

  const set = (args: { mode?: string; monitor?: number }) => {
    invoke("set_window", args).catch(onError);
    onClose();
  };

  return (
    <Menu anchor={anchor} className="window-menu" onClose={onClose}>
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
    </Menu>
  );
}
