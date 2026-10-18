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

/** What to call a session: the name given in the sidebar, its title, its first prompt. */
export function sessionName(s: Session): string {
  return s.user_title ?? s.title ?? s.first_prompt ?? "New session";
}

export function projectName(s: Session): string {
  return s.project?.name ?? s.cwd?.split("/").pop() ?? s.session_id.slice(0, 8);
}

export function projectKey(s: Session): string {
  return s.project?.root ?? s.cwd ?? s.session_id;
}

/** "3:20 pm" today, else "Tue 9:00 am". */
export function resetTime(ms: number, now: number): string {
  const d = new Date(ms);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase();
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return sameDay ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

/** Bar colour for a usage percentage. */
export function usageLevel(percent: number): "ok" | "high" | "critical" {
  return percent >= 90 ? "critical" : percent >= 70 ? "high" : "ok";
}
