// Mirrors src-tauri/src/session.rs (serde output).
export type Status =
  "starting" | "working" | "needs_permission" | "needs_input" | "idle" | "error" | "closed";

export type AlertKind =
  "denied" | "stop_failure" | "repeated_failure" | "waiting" | "slow" | "compact";

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
  alerts: Alert[];
  closed_reason: string | null;
  starred: boolean;
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
