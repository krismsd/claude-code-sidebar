import type { AlertKind, Session, Status } from "./types";

export const STATUS_LABEL: Record<Status, string> = {
  starting: "new",
  working: "working",
  needs_permission: "permission",
  needs_input: "your turn",
  idle: "ready",
  error: "error",
  closed: "closed",
};

export const ALERT_EMOJI: Record<AlertKind, string> = {
  denied: "🚫",
  stop_failure: "⚠️",
  repeated_failure: "🔁",
  waiting: "⏳",
  slow: "🐌",
  compact: "📦",
};

const CLOSED_REASON: Record<string, string> = {
  prompt_input_exit: "exited",
  other: "tab closed",
  logout: "logged out",
  lost: "process gone",
};

export function closedLabel(s: Session): string {
  return CLOSED_REASON[s.closed_reason ?? ""] ?? "closed";
}

/** Largest unit only: "42s", "5m", "3h", "2d". */
export function elapsed(fromMs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor((nowMs - fromMs) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** What to call a session: its title, else its first prompt, else "New session". */
export function sessionName(s: Session): string {
  return s.title ?? s.first_prompt ?? "New session";
}

export function projectName(s: Session): string {
  return s.project?.name ?? s.cwd?.split("/").pop() ?? s.session_id.slice(0, 8);
}

export function projectKey(s: Session): string {
  return s.project?.root ?? s.cwd ?? s.session_id;
}
