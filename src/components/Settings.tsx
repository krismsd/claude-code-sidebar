import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Config, Monitor, ProjectOverride, Settings, TasksConfig } from "../lib/types";

type Tab = "general" | "alerts" | "projects" | "advanced";
type TaskSource = "none" | "connector" | "cli";

const TABS: [Tab, string][] = [
  ["general", "General"],
  ["alerts", "Alerts"],
  ["projects", "Projects"],
  ["advanced", "Advanced"],
];

const SOUND_KINDS: [string, string][] = [
  ["denied", "🚫 Auto-mode denial"],
  ["stop_failure", "⚠️ API error"],
  ["repeated_failure", "🔁 Same tool keeps failing"],
  ["waiting", "⏳ Waiting on you"],
  ["finished", "✅ Background job finished"],
  ["slow", "🐌 Silent while working"],
  ["compact", "📦 Compacting context"],
  ["pr", "🔀 PR checks failed, changes requested or merged"],
  ["permission", "New permission prompt"],
];

interface SoundChoice {
  value: string;
  label: string;
  /** For a sound-scheme alias: the file it plays ("" = nothing). */
  plays: string | null;
}
interface SoundChoices {
  files: SoundChoice[];
  aliases: SoundChoice[];
}
const CUSTOM = "__custom";

const blank = (v: string | null | undefined) => (v && v.trim() ? v.trim() : undefined);

/** What gets saved: empty fields dropped, entries with nothing left removed. */
function cleanProjects(projects: ProjectOverride[]): ProjectOverride[] {
  return projects
    .map((p) => {
      const t = p.tasks;
      const tasks: TasksConfig | undefined = t
        ? {
            connector: blank(t.connector),
            query: t.query.trim(),
            tools: (t.tools ?? []).map((x) => x.trim()).filter(Boolean),
            prompt: blank(t.prompt),
          }
        : undefined;
      return {
        path: p.path,
        name: blank(p.name),
        color: blank(p.color),
        emoji: blank(p.emoji),
        label: blank(p.label),
        tasks,
      };
    })
    .filter((p) => p.name || p.color || p.emoji || p.label || p.tasks);
}

function sourceOf(t: TasksConfig | null | undefined): TaskSource {
  if (!t) return "none";
  return t.connector ? "connector" : "cli";
}

/** The settings window (`?view=settings`): edits config.toml and projects.toml. */
export function SettingsWindow() {
  const [saved, setSaved] = useState<Settings | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [projects, setProjects] = useState<ProjectOverride[]>([]);
  const [sources, setSources] = useState<Record<string, TaskSource>>({});
  const [tab, setTab] = useState<Tab>(
    () => (new URLSearchParams(window.location.search).get("tab") as Tab | null) ?? "general",
  );
  const [status, setStatus] = useState<{ error: boolean; text: string } | null>(null);
  const [monitors, setMonitors] = useState<Monitor[]>([]);

  const apply = useCallback((s: Settings) => {
    setSaved(s);
    setConfig(s.config);
    setProjects(s.projects);
    setSources(Object.fromEntries(s.projects.map((p) => [p.path, sourceOf(p.tasks)])));
  }, []);
  const load = useCallback(() => invoke<Settings>("get_settings").then(apply), [apply]);

  useEffect(() => {
    invoke<Settings>("get_settings")
      .then(apply)
      .catch((e) => setStatus({ error: true, text: String(e) }));
    invoke<Monitor[]>("list_monitors")
      .then(setMonitors)
      .catch(() => {});
  }, [apply]);

  const dirty = useMemo(
    () =>
      !!saved &&
      !!config &&
      (JSON.stringify(config) !== JSON.stringify(saved.config) ||
        JSON.stringify(cleanProjects(projects)) !== JSON.stringify(cleanProjects(saved.projects))),
    [saved, config, projects],
  );

  useEffect(() => {
    const un = listen<Tab>("settings-tab", (e) => setTab(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Edited by hand (or from the sidebar's ⚙ menu) while open: pick that up unless we have
  // unsaved changes of our own.
  useEffect(() => {
    const un = listen("config-reloaded", () => {
      if (!dirty) load().catch(() => {});
    });
    return () => {
      un.then((f) => f());
    };
  }, [dirty, load]);

  const save = useCallback(async () => {
    if (!config) return;
    try {
      const launch_env = Object.fromEntries(
        Object.entries(config.launch_env).filter(([k]) => k.trim()),
      );
      await invoke("save_settings", {
        config: { ...config, launch_env },
        projects: cleanProjects(projects),
      });
      await load();
      setStatus({ error: false, text: "Saved. Changes apply in a couple of seconds." });
    } catch (e) {
      setStatus({ error: true, text: String(e) });
    }
  }, [config, projects, load]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        save();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [save]);

  if (!saved || !config) return <main className="settings" />;

  const set = <K extends keyof Config>(key: K, value: Config[K]) => {
    setStatus(null);
    setConfig({ ...config, [key]: value });
  };
  const setProject = (path: string, change: Partial<ProjectOverride>) => {
    setStatus(null);
    setProjects((ps) =>
      ps.some((p) => p.path === path)
        ? ps.map((p) => (p.path === path ? { ...p, ...change } : p))
        : [...ps, { path, ...change }],
    );
  };

  return (
    <main className="settings">
      <nav className="settings-tabs">
        {TABS.map(([id, label]) => (
          <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </nav>
      <div className="settings-body">
        {tab === "general" && <General config={config} set={set} monitors={monitors} />}
        {tab === "alerts" && <Alerts config={config} set={set} />}
        {tab === "projects" && (
          <Projects
            settings={saved}
            projects={projects}
            sources={sources}
            setSource={(path, s) => {
              setSources({ ...sources, [path]: s });
              setProject(path, {
                tasks:
                  s === "none"
                    ? null
                    : {
                        query: "",
                        ...projects.find((p) => p.path === path)?.tasks,
                        connector: s === "connector" ? "Atlassian" : "",
                      },
              });
            }}
            setProject={setProject}
            remove={(path) => {
              setProjects(projects.filter((p) => p.path !== path));
              setSources({ ...sources, [path]: "none" });
            }}
          />
        )}
        {tab === "advanced" && <Advanced config={config} set={set} settings={saved} />}
      </div>
      <footer className="settings-footer">
        <span className={status?.error ? "status error" : "status"}>
          {status?.text ?? (dirty ? "Unsaved changes" : "")}
        </span>
        <button
          disabled={!dirty}
          onClick={() => {
            setStatus(null);
            load();
          }}
        >
          Revert
        </button>
        <button className="primary" disabled={!dirty} onClick={save}>
          Save
        </button>
      </footer>
    </main>
  );
}

type SetConfig = <K extends keyof Config>(key: K, value: Config[K]) => void;

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <span className="field-control">
        {children}
        {hint && <span className="field-hint">{hint}</span>}
      </span>
    </label>
  );
}

function NumberInput({
  value,
  min,
  max,
  onChange,
  unit,
}: {
  value: number;
  min: number;
  max?: number;
  onChange: (n: number) => void;
  unit?: string;
}) {
  return (
    <span className="number">
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        onChange={(e) => {
          const n = Math.round(Number(e.target.value));
          if (Number.isFinite(n)) onChange(Math.max(min, max === undefined ? n : Math.min(max, n)));
        }}
      />
      {unit && <span className="muted">{unit}</span>}
    </span>
  );
}

function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: [T, string][];
  onChange: (v: T) => void;
}) {
  return (
    <span className="segmented">
      {options.map(([v, label]) => (
        <button
          key={v}
          type="button"
          className={value === v ? "active" : ""}
          onClick={() => onChange(v)}
        >
          {label}
        </button>
      ))}
    </span>
  );
}

function General({
  config,
  set,
  monitors,
}: {
  config: Config;
  set: SetConfig;
  monitors: Monitor[];
}) {
  const wt = config.terminal_window;
  const wtKind = wt === "0" || wt === "new" ? wt : "named";
  return (
    <>
      <h2>Window</h2>
      <Field
        label="Position"
        hint={
          config.mode === "docked"
            ? "Reserves its strip of the screen, like the taskbar."
            : "A normal window you can move. Reserves no space."
        }
      >
        <Segmented
          value={config.mode}
          options={[
            ["docked", "Docked"],
            ["floating", "Floating"],
          ]}
          onChange={(v) => set("mode", v)}
        />
      </Field>
      <Field label="Monitor">
        <select value={config.monitor} onChange={(e) => set("monitor", Number(e.target.value))}>
          <option value={0}>Primary monitor</option>
          {monitors.map((m) => (
            <option key={m.number} value={m.number}>
              Monitor {m.number}: {m.width}×{m.height} at {m.scale_percent}%
              {m.primary ? " (primary)" : ""}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Edge">
        <Segmented
          value={config.edge}
          options={[
            ["left", "Left"],
            ["right", "Right"],
          ]}
          onChange={(v) => set("edge", v)}
        />
      </Field>
      <Field label="Width">
        <span className="range">
          <input
            type="range"
            min={240}
            max={600}
            step={10}
            value={config.width}
            onChange={(e) => set("width", Number(e.target.value))}
          />
          <NumberInput
            value={config.width}
            min={200}
            max={900}
            unit="px"
            onChange={(n) => set("width", n)}
          />
        </span>
      </Field>
      <Field label="Start with Windows">
        <input
          type="checkbox"
          checked={config.autostart}
          onChange={(e) => set("autostart", e.target.checked)}
        />
      </Field>

      <h2>Sessions</h2>
      <Field
        label="Run sessions in"
        hint={
          config.session_mode === "terminal"
            ? "Closing a tab ends its session."
            : config.session_mode === "tmux"
              ? "Sessions run in tmux: closing a tab keeps them running, and a click brings them back."
              : "Sessions run in tmux and show in the sidebar's own terminal window. Closing it keeps them running."
        }
      >
        <select
          value={config.session_mode}
          onChange={(e) => set("session_mode", e.target.value as Config["session_mode"])}
        >
          <option value="terminal">Windows Terminal tabs</option>
          <option value="tmux">tmux, shown in Windows Terminal</option>
          <option value="window">tmux, shown in the sidebar's own window</option>
        </select>
      </Field>
      <Field label="New and resumed tabs open in">
        <select
          value={wtKind}
          onChange={(e) =>
            set("terminal_window", e.target.value === "named" ? "claude" : e.target.value)
          }
        >
          <option value="0">The last used Terminal window</option>
          <option value="new">A new window each time</option>
          <option value="named">A window with a name…</option>
        </select>
        {wtKind === "named" && (
          <input
            type="text"
            value={wt}
            placeholder="window name"
            onChange={(e) => set("terminal_window", e.target.value)}
          />
        )}
      </Field>
      <Field label="Keep closed sessions for">
        <NumberInput
          value={config.closed_retention_days}
          min={1}
          unit="days after you remove them"
          onChange={(n) => set("closed_retention_days", n)}
        />
      </Field>
    </>
  );
}

/** Records a key combination such as "Ctrl+Alt+J". Backspace clears it. */
function HotkeyInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <input
      type="text"
      readOnly
      className="hotkey"
      value={value}
      placeholder="Off. Click and press keys"
      onKeyDown={(e) => {
        if (e.key === "Tab") return;
        e.preventDefault();
        const mods = [
          e.ctrlKey && "Ctrl",
          e.altKey && "Alt",
          e.shiftKey && "Shift",
          e.metaKey && "Super",
        ].filter(Boolean);
        if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return;
        if (mods.length === 0 && (e.key === "Backspace" || e.key === "Delete")) {
          onChange("");
          return;
        }
        if (mods.length === 0 && e.key === "Escape") {
          e.currentTarget.blur();
          return;
        }
        if (mods.length === 0) return; // a bare key would steal it from every app
        const key = e.code.replace(/^Key/, "").replace(/^Digit/, "");
        onChange([...mods, key].join("+"));
      }}
    />
  );
}

function SoundPicker({
  value,
  choices,
  onChange,
}: {
  value: string;
  choices: SoundChoices;
  onChange: (v: string) => void;
}) {
  const known =
    value === "" || [...choices.files, ...choices.aliases].some((c) => c.value === value);
  return (
    <span className="sound">
      <select
        value={known ? value : CUSTOM}
        onChange={async (e) => {
          if (e.target.value !== CUSTOM) return onChange(e.target.value);
          const picked = await invoke<string | null>("pick_sound");
          if (picked) onChange(picked);
        }}
      >
        <option value="">None</option>
        <optgroup label="Windows sounds">
          {choices.files.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </optgroup>
        {/* On Windows 11 most of these play the same file, so say which. */}
        <optgroup label="Your sound scheme">
          {choices.aliases.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label} ({c.plays || "silent"})
            </option>
          ))}
        </optgroup>
        <option value={CUSTOM}>{known ? "Another .wav file…" : value.split("\\").pop()}</option>
      </select>
      <button
        type="button"
        title="Play"
        disabled={!value}
        onClick={() => invoke("play_sound", { sound: value })}
      >
        ▶
      </button>
    </span>
  );
}

function Alerts({ config, set }: { config: Config; set: SetConfig }) {
  const [choices, setChoices] = useState<SoundChoices>({ files: [], aliases: [] });
  useEffect(() => {
    invoke<SoundChoices>("list_sounds")
      .then(setChoices)
      .catch(() => {});
  }, []);
  return (
    <>
      <h2>When to alert</h2>
      <Field label="⏳ Waiting on you after">
        <NumberInput
          value={config.waiting_minutes}
          min={1}
          unit="minutes"
          onChange={(n) => set("waiting_minutes", n)}
        />
      </Field>
      <Field label="🐌 Silent while working after">
        <NumberInput
          value={config.slow_minutes}
          min={1}
          unit="minutes"
          onChange={(n) => set("slow_minutes", n)}
        />
      </Field>
      <Field label="Windows notifications">
        <select
          value={config.toasts}
          onChange={(e) => set("toasts", e.target.value as Config["toasts"])}
        >
          <option value="when_hidden">Only when a full-screen app covers the sidebar</option>
          <option value="always">Always</option>
          <option value="never">Never</option>
        </select>
      </Field>
      <Field
        label="Jump to waiting session"
        hint="Focuses the session that has waited on you longest."
      >
        <HotkeyInput value={config.jump_hotkey} onChange={(v) => set("jump_hotkey", v)} />
      </Field>

      <h2>Sounds</h2>
      {SOUND_KINDS.map(([kind, label]) => (
        <Field key={kind} label={label}>
          <SoundPicker
            value={config.sounds[kind] ?? ""}
            choices={choices}
            onChange={(v) => set("sounds", { ...config.sounds, [kind]: v })}
          />
        </Field>
      ))}
    </>
  );
}

function Projects({
  settings,
  projects,
  sources,
  setSource,
  setProject,
  remove,
}: {
  settings: Settings;
  projects: ProjectOverride[];
  sources: Record<string, TaskSource>;
  setSource: (path: string, s: TaskSource) => void;
  setProject: (path: string, change: Partial<ProjectOverride>) => void;
  remove: (path: string) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const tracked = new Map(settings.tracked.map((t) => [t.root, t]));
  const paths = [
    ...settings.tracked.map((t) => t.root),
    ...projects.map((p) => p.path).filter((p) => !tracked.has(p)),
  ];
  if (paths.length === 0)
    return <p className="muted">No projects yet. Add one from the sidebar's + button.</p>;
  return (
    <>
      <p className="muted intro">
        How each project looks in the sidebar, and where its tasks come from. Leave a field empty to
        use the default.
      </p>
      {paths.map((path) => {
        const p = projects.find((x) => x.path === path) ?? { path };
        const def = tracked.get(path);
        const color = p.color || def?.color || "#888";
        const source = sources[path] ?? sourceOf(p.tasks);
        const expanded = open === path;
        const update = (c: Partial<ProjectOverride>) => setProject(path, c);
        const updateTasks = (c: Partial<TasksConfig>) =>
          update({ tasks: { query: "", ...p.tasks, ...c } });
        return (
          <section key={path} className={expanded ? "project open" : "project"}>
            <button className="project-head" onClick={() => setOpen(expanded ? null : path)}>
              <span className="dot" style={{ background: color }} />
              <span className="project-title">
                {p.emoji && `${p.emoji} `}
                {p.name || def?.name || path.split("/").pop()}
                {p.label && <span className="muted"> · {p.label}</span>}
              </span>
              <span className="muted small path">{path}</span>
              {!def && <span className="badge">not in sidebar</span>}
              <span className="chevron">{expanded ? "▾" : "▸"}</span>
            </button>
            {expanded && (
              <div className="project-body">
                <Field label="Name">
                  <input
                    type="text"
                    value={p.name ?? ""}
                    placeholder={def?.name ?? path.split("/").pop()}
                    onChange={(e) => update({ name: e.target.value })}
                  />
                </Field>
                <Field label="Label" hint="Shown next to the name, e.g. a team.">
                  <input
                    type="text"
                    value={p.label ?? ""}
                    onChange={(e) => update({ label: e.target.value })}
                  />
                </Field>
                <Field label="Emoji">
                  <input
                    type="text"
                    className="emoji"
                    value={p.emoji ?? ""}
                    onChange={(e) => update({ emoji: e.target.value })}
                  />
                </Field>
                <Field label="Colour">
                  <span className="swatches">
                    <button
                      type="button"
                      className={!p.color ? "swatch auto active" : "swatch auto"}
                      title="Automatic"
                      style={{ background: def?.color }}
                      onClick={() => update({ color: null })}
                    >
                      A
                    </button>
                    {settings.palette.map((c) => (
                      <button
                        key={c}
                        type="button"
                        className={p.color === c ? "swatch active" : "swatch"}
                        style={{ background: c }}
                        title={c}
                        onClick={() => update({ color: c })}
                      />
                    ))}
                    <input
                      type="color"
                      value={color}
                      title="Any colour"
                      onChange={(e) => update({ color: e.target.value })}
                    />
                  </span>
                </Field>

                <h3>Tasks</h3>
                <Field label="Source">
                  <Segmented
                    value={source}
                    options={[
                      ["none", "None"],
                      ["connector", "claude.ai connector"],
                      ["cli", "Command-line tool"],
                    ]}
                    onChange={(s) => setSource(path, s)}
                  />
                </Field>
                {source !== "none" && (
                  <>
                    {source === "connector" && (
                      <Field label="Connector" hint="Its name in claude.ai, e.g. Atlassian.">
                        <input
                          type="text"
                          value={p.tasks?.connector ?? ""}
                          onChange={(e) => updateTasks({ connector: e.target.value })}
                        />
                      </Field>
                    )}
                    <Field
                      label="What to list"
                      hint="Plain English. Claude turns it into the search."
                    >
                      <textarea
                        rows={4}
                        value={p.tasks?.query ?? ""}
                        placeholder={
                          source === "connector"
                            ? "Jira issues in project ACME assigned to me that aren't done"
                            : "Azure DevOps work items assigned to me that aren't done, via az boards query"
                        }
                        onChange={(e) => updateTasks({ query: e.target.value })}
                      />
                    </Field>
                    <Field
                      label="Allowed tools"
                      hint={
                        source === "connector"
                          ? "One per line. Leave empty for the connector's read-only defaults."
                          : "Permission rules, one per line, e.g. Bash(az boards query:*)"
                      }
                    >
                      <textarea
                        rows={2}
                        value={(p.tasks?.tools ?? []).join("\n")}
                        onChange={(e) => updateTasks({ tools: e.target.value.split("\n") })}
                      />
                    </Field>
                    <Field
                      label="First prompt"
                      hint="For sessions started from a task: {id} {title} {url} {summary} {status}"
                    >
                      <textarea
                        rows={3}
                        value={p.tasks?.prompt ?? ""}
                        placeholder={"Work on {id}: {title}\n\n{summary}\n\n{url}"}
                        onChange={(e) => updateTasks({ prompt: e.target.value })}
                      />
                    </Field>
                  </>
                )}
                <div className="project-actions">
                  <button type="button" onClick={() => remove(path)}>
                    {def ? "Reset to defaults" : "Remove these settings"}
                  </button>
                </div>
              </div>
            )}
          </section>
        );
      })}
    </>
  );
}

function Advanced({
  config,
  set,
  settings,
}: {
  config: Config;
  set: SetConfig;
  settings: Settings;
}) {
  const env = Object.entries(config.launch_env);
  const setEnv = (entries: [string, string][]) => set("launch_env", Object.fromEntries(entries));
  return (
    <>
      <h2>WSL</h2>
      <Field label="Distro" hint="For sessions that don't report one.">
        <input type="text" value={config.distro} onChange={(e) => set("distro", e.target.value)} />
      </Field>
      <Field label="Launcher" hint="Command that starts Claude Code for new and resumed sessions.">
        <input
          type="text"
          value={config.launcher}
          onChange={(e) => set("launcher", e.target.value)}
        />
      </Field>
      <Field label="Environment for launched sessions">
        <span className="env">
          {env.map(([k, v], i) => (
            <span className="env-row" key={i}>
              <input
                type="text"
                value={k}
                placeholder="NAME"
                onChange={(e) => setEnv(env.map((x, j) => (j === i ? [e.target.value, x[1]] : x)))}
              />
              <input
                type="text"
                value={v}
                placeholder="value"
                onChange={(e) => setEnv(env.map((x, j) => (j === i ? [x[0], e.target.value] : x)))}
              />
              <button
                type="button"
                title="Remove"
                onClick={() => setEnv(env.filter((_, j) => j !== i))}
              >
                ×
              </button>
            </span>
          ))}
          <button
            type="button"
            className="add"
            disabled={"" in config.launch_env}
            onClick={() => setEnv([...env, ["", ""]])}
          >
            + Add variable
          </button>
        </span>
      </Field>

      <h2>Claude helpers</h2>
      <p className="muted intro">Commands run inside WSL. Leave one empty to turn it off.</p>
      <Field label="Recaps" hint="A one-line Haiku summary after each turn.">
        <input
          type="text"
          value={config.recap_command}
          placeholder="off"
          onChange={(e) => set("recap_command", e.target.value)}
        />
      </Field>
      <Field label="At most one recap every">
        <NumberInput
          value={config.recap_min_interval_s}
          min={0}
          unit="seconds per session"
          onChange={(n) => set("recap_min_interval_s", n)}
        />
      </Field>
      <Field
        label="Usage limits at startup"
        hint="One tiny Haiku call. They also update with each recap."
      >
        <input
          type="text"
          value={config.usage_command}
          placeholder="off"
          onChange={(e) => set("usage_command", e.target.value)}
        />
      </Field>
      <Field label="Task lists">
        <input
          type="text"
          value={config.tasks_command}
          placeholder="off"
          onChange={(e) => set("tasks_command", e.target.value)}
        />
      </Field>

      <h2>Files</h2>
      <p className="muted intro">Everything here is saved to these files, comments kept.</p>
      <div className="files">
        <button type="button" onClick={() => invoke("open_settings_file")}>
          Open config.toml
        </button>
        <button type="button" onClick={() => invoke("open_projects_file")}>
          Open projects.toml
        </button>
        <span className="muted small">{settings.config_path.replace(/\\config\.toml$/, "")}</span>
      </div>
    </>
  );
}
