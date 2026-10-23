import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { ClipboardAddon } from "@xterm/addon-clipboard";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { openUrl } from "@tauri-apps/plugin-opener";
import "@xterm/xterm/css/xterm.css";
import type { Session, UiState } from "../lib/types";
import { STATUS_LABEL, sessionName } from "../lib/format";

interface Tab {
  /** tmux session name, "cc-<tag>". */
  name: string;
}

/** Windows Terminal's default ("Campbell") colours, so sessions look as they did there. */
const THEME = {
  background: "#0c0c0c",
  foreground: "#cccccc",
  cursor: "#ffffff",
  selectionBackground: "#264f78",
  black: "#0c0c0c",
  red: "#c50f1f",
  green: "#13a10e",
  yellow: "#c19c00",
  blue: "#0037da",
  magenta: "#881798",
  cyan: "#3a96dd",
  white: "#cccccc",
  brightBlack: "#767676",
  brightRed: "#e74856",
  brightGreen: "#16c60c",
  brightYellow: "#f9f1a5",
  brightBlue: "#3b78ff",
  brightMagenta: "#b4009e",
  brightCyan: "#61d6d6",
  brightWhite: "#f2f2f2",
};

/** Output for each open view, by tmux name (events arrive for every view). */
const terminals = new Map<string, Terminal>();

/** The sidebar's own terminal window ("window" session mode): one tab per tmux-hosted
 * session, each an xterm.js view attached through `term_open`. Closing a tab detaches;
 * Claude keeps running and the sidebar card says so. */
export function TerminalWindow() {
  const [tabs, setTabs] = useState<Tab[]>(() => {
    const open = new URLSearchParams(window.location.search).get("open");
    return open ? [{ name: open }] : [];
  });
  const [active, setActive] = useState<string | null>(tabs[0]?.name ?? null);
  const [state, setState] = useState<UiState | null>(null);

  useEffect(() => {
    invoke<UiState>("get_state").then(setState);
    const subs = [
      listen<UiState>("state", (e) => setState(e.payload)),
      listen<{ name: string }>("term-open", ({ payload: { name } }) => {
        setTabs((t) => (t.some((x) => x.name === name) ? t : [...t, { name }]));
        setActive(name);
      }),
      listen<{ name: string; data: string }>("term-output", ({ payload }) =>
        terminals.get(payload.name)?.write(payload.data),
      ),
      // The session ended, or was detached elsewhere: its tab goes.
      listen<string>("term-exit", ({ payload: name }) => {
        setTabs((t) => t.filter((x) => x.name !== name));
      }),
    ];
    return () => subs.forEach((s) => s.then((f) => f()));
  }, []);

  // Keep a tab active when the active one goes.
  const shown = active && tabs.some((t) => t.name === active) ? active : (tabs[0]?.name ?? null);
  const byName = (name: string) => state?.sessions.find((s) => s.tmux === name);
  const current = shown ? byName(shown) : undefined;

  useEffect(() => {
    document.title = current ? `${sessionName(current)} · Claude Code` : "Claude Code";
  }, [current]);

  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const w = getCurrentWindow();
    const check = () => void w.isMaximized().then(setMaximized);
    check();
    const sub = w.onResized(check);
    return () => void sub.then((f) => f());
  }, []);

  const close = (name: string) => {
    invoke("term_close", { name });
    setTabs((t) => t.filter((x) => x.name !== name));
  };

  return (
    <main className="term-window">
      <nav className="term-tabs" data-tauri-drag-region>
        {tabs.map(({ name }) => {
          const s = byName(name);
          return (
            <div
              key={name}
              className={`term-tab${name === shown ? " active" : ""}`}
              style={{ "--project": s?.project?.color ?? "#888" } as CSSProperties}
              onClick={() => setActive(name)}
              onAuxClick={(e) => e.button === 1 && close(name)}
              title={s ? `${s.project?.name ?? ""}\n${sessionName(s)}` : name}
            >
              <span className="term-tab-name">{s ? sessionName(s) : name}</span>
              {s && <span className={`chip chip-${s.status}`}>{STATUS_LABEL[s.status]}</span>}
              <button
                title="Close (the session keeps running)"
                onClick={(e) => (e.stopPropagation(), close(name))}
              >
                {"\uE8BB"}
              </button>
            </div>
          );
        })}
        <div className="term-drag" data-tauri-drag-region />
        <WindowButtons maximized={maximized} />
      </nav>
      <div className="term-panes">
        {tabs.length === 0 && (
          <p className="term-empty">
            No sessions open. Click a card in the sidebar to show it here.
          </p>
        )}
        {tabs.map(({ name }) => (
          <Pane key={name} name={name} active={name === shown} session={byName(name)} />
        ))}
      </div>
    </main>
  );
}

/** Minimise, maximise/restore and close, drawn like Windows' own (Segoe Fluent Icons). */
function WindowButtons({ maximized }: { maximized: boolean }) {
  const w = getCurrentWindow();
  return (
    <div className="caption-buttons">
      <button title="Minimise" onClick={() => w.minimize()}>
        {"\uE921"}
      </button>
      <button title={maximized ? "Restore" : "Maximise"} onClick={() => w.toggleMaximize()}>
        {maximized ? "\uE923" : "\uE922"}
      </button>
      <button className="caption-close" title="Close" onClick={() => w.close()}>
        {"\uE8BB"}
      </button>
    </div>
  );
}

function Pane({ name, active, session }: { name: string; active: boolean; session?: Session }) {
  const ref = useRef<HTMLDivElement>(null);
  const fit = useRef<FitAddon | null>(null);
  const term = useRef<Terminal | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const t = new Terminal({
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 14,
      theme: THEME,
      cursorBlink: true,
      scrollback: 1000,
      allowProposedApi: true,
      macOptionIsMeta: true,
      // Hyperlinks the program marks (OSC 8; Claude Code uses them and tmux passes them on).
      // xterm.js's default asks with confirm() and calls window.open, neither of which does
      // anything in the app's webview: open them in the default browser instead.
      linkHandler: { activate: (_, uri) => void openUrl(uri) },
    });
    const f = new FitAddon();
    t.loadAddon(f);
    // Character widths as tmux sees them (emoji are two columns). With xterm.js's default
    // (Unicode 6) table, partial redraws of lines with emoji, such as Claude's status line,
    // land a column off and leave fragments behind.
    t.loadAddon(new Unicode11Addon());
    t.unicode.activeVersion = "11";
    t.loadAddon(new WebLinksAddon((_, url) => openUrl(url)));
    // tmux copies through OSC 52 (set-clipboard on).
    t.loadAddon(new ClipboardAddon());
    t.open(el);
    try {
      t.loadAddon(new WebglAddon());
    } catch {
      // No WebGL: the DOM renderer is slower but fine.
    }
    f.fit();
    terminals.set(name, t);
    term.current = t;
    fit.current = f;

    const send = (data: string) => invoke("term_write", { name, data });
    t.onData(send);
    t.attachCustomKeyEventHandler((e) => {
      // Shift+Enter: a newline in the prompt. xterm.js would send a plain Enter (from the
      // keypress too, so block every event type), so send the modified key in xterm's
      // modifyOtherKeys form, which tmux passes on as Shift+Enter.
      if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.altKey) {
        if (e.type === "keydown") send("\x1b[27;2;13~");
        return false;
      }
      if (e.type !== "keydown") return true;
      const ctrl = e.ctrlKey && !e.altKey;
      // Copy a selection with Ctrl+C (without one, Ctrl+C interrupts as usual).
      if (ctrl && e.key.toLowerCase() === "c" && t.hasSelection()) {
        navigator.clipboard.writeText(t.getSelection());
        t.clearSelection();
        return false;
      }
      // Ctrl+V: let the browser paste (handled below), rather than sending ^V.
      if (ctrl && e.key.toLowerCase() === "v") return false;
      return true;
    });
    // A paste with no text (an image): send ^V, so Claude Code reads the clipboard itself.
    const onPaste = (e: ClipboardEvent) => {
      if (!e.clipboardData?.getData("text/plain")) {
        e.preventDefault();
        send("\x16");
      }
    };
    el.addEventListener("paste", onPaste, true);

    invoke("term_open", { name, cols: t.cols, rows: t.rows }).catch((e) =>
      t.write(`\r\n[couldn't open ${name}: ${e}]\r\n`),
    );
    const resize = () => {
      if (!el.offsetParent) return; // hidden tab
      f.fit();
      invoke("term_resize", { name, cols: t.cols, rows: t.rows }).catch(() => {});
    };
    const ro = new ResizeObserver(resize);
    ro.observe(el);
    return () => {
      ro.disconnect();
      el.removeEventListener("paste", onPaste, true);
      terminals.delete(name);
      t.dispose();
    };
  }, [name]);

  // Shown again: fit to the window (it may have been resized meanwhile) and take focus.
  useEffect(() => {
    if (!active) return;
    requestAnimationFrame(() => {
      fit.current?.fit();
      const t = term.current;
      if (t) invoke("term_resize", { name, cols: t.cols, rows: t.rows }).catch(() => {});
      t?.focus();
    });
  }, [active, name]);

  return (
    <div
      className="term-pane"
      ref={ref}
      style={{ display: active ? "block" : "none" }}
      data-session={session?.session_id}
    />
  );
}
