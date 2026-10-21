// Mirrors src-tauri/src/session.rs (serde output).
export type Status =
  "starting" | "working" | "needs_permission" | "needs_input" | "idle" | "error" | "closed";

export type AlertKind =
  "denied" | "stop_failure" | "repeated_failure" | "waiting" | "slow" | "compact" | "finished";

export interface Alert {
  id: number;
  kind: AlertKind;
  text: string;
  at: number;
}

export interface PendingPermission {
  tool: string;
  summary: string;
  since: number;
  answerable: boolean;
}

export interface Project {
  root: string;
  name: string;
  color: string;
  emoji: string | null;
  label: string | null;
  worktree: string | null;
  branch: string | null;
}

export interface Session {
  session_id: string;
  tag: string | null;
  pid: number | null;
  wt_session: string | null;
  distro: string | null;
  /** tmux session it runs in ("cc-<tag>"), when hosted in tmux. */
  tmux: string | null;
  /** For a tmux session: a terminal shows it (null until checked). */
  attached: boolean | null;
  cwd: string | null;
  shell_cwd: string | null;
  project: Project | null;
  transcript_path: string | null;
  permission_mode: string | null;
  model: string | null;
  status: Status;
  status_since: number;
  started_at: number;
  last_event_at: number;
  current_tool: string | null;
  last_prompt: string | null;
  first_prompt: string | null;
  title: string | null;
  user_title: string | null;
  user_title_synced: boolean;
  start_source: string | null;
  recap: string | null;
  recap_haiku: string | null;
  recap_detail: string | null;
  stop_seq: number;
  pending_permission: PendingPermission | null;
  subagents_running: number;
  background_running: number;
  /** What runs in the background, as of the last Stop/SubagentStop. */
  background: Background[];
  alerts: Alert[];
  closed_reason: string | null;
  /** Removed by the user: in its project's "closed" section (null: not yet known). */
  removed: boolean | null;
}

export interface Recent {
  path: string;
  name: string;
  color: string;
  last_used: number;
}

export type SessionAction = "end" | "folder" | "vscode";

/** A tracked project (SPEC §3.12); `project` has no branch/worktree. */
export interface Tracked {
  project: Project;
  added_at: number;
  /** Put away in the drawer. */
  shelved: boolean;
  /** Session ids in the order the user dragged them into. */
  session_order: string[];
}

export interface UiState {
  projects: Tracked[];
  sessions: Session[];
  /** Roots of projects with tasks set up in projects.toml. */
  task_projects: string[];
  collapsed: boolean;
  floating: boolean;
  left_edge: boolean;
  /** 0 = primary, n = nth monitor from the left. */
  monitor: number;
  usage: Usage | null;
}

export interface UsageWindow {
  /** 0–100 */
  percent: number;
  /** epoch ms */
  resets_at: number | null;
}

/** Subscription usage limits (what /usage shows). */
export interface Usage {
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  status: string;
  using_overage: boolean;
  updated_at: number;
}

export interface Monitor {
  number: number;
  name: string;
  primary: boolean;
  width: number;
  height: number;
  scale_percent: number;
}

/** A task from an issue tracker (SPEC §3.12). */
export interface Task {
  id: string;
  title: string;
  status: string;
  url: string;
  summary: string;
}

// Mirrors src-tauri/src/config.rs and project.rs (settings window).
export interface Config {
  mode: "docked" | "floating";
  monitor: number;
  edge: "right" | "left";
  width: number;
  distro: string;
  launcher: string;
  launch_env: Record<string, string>;
  terminal_window: string;
  waiting_minutes: number;
  slow_minutes: number;
  closed_retention_days: number;
  sounds: Record<string, string>;
  toasts: "when_hidden" | "always" | "never";
  jump_hotkey: string;
  recap_command: string;
  recap_min_interval_s: number;
  usage_command: string;
  tasks_command: string;
  autostart: boolean;
  session_mode: "terminal" | "tmux" | "window";
}

export interface TasksConfig {
  connector?: string;
  query: string;
  tools?: string[];
  prompt?: string | null;
}

/** One `[[project]]` in projects.toml. */
export interface ProjectOverride {
  path: string;
  name?: string | null;
  color?: string | null;
  emoji?: string | null;
  label?: string | null;
  tasks?: TasksConfig | null;
}

export interface Settings {
  config: Config;
  projects: ProjectOverride[];
  tracked: { root: string; name: string; color: string }[];
  palette: string[];
  config_path: string;
  projects_path: string;
}

/** Something running in the background of a session (from Stop/SubagentStop). */
export interface Background {
  /** Claude Code's task id; its output file is read for `last_line`/`url`. */
  id: string;
  agent: boolean;
  description: string;
  command: string;
  /** A job Claude will act on when it finishes (vs a service such as a dev server). */
  awaited: boolean;
  /** Latest line of its output. */
  last_line: string | null;
}
