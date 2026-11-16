import type { ReactNode } from "react";
import type { Usage, UsageWindow } from "../lib/types";
import { elapsed, resetTime, usageLevel, usageNow } from "../lib/format";

interface Props {
  usage: Usage | null;
  now: number;
  /** The sidebar's controls (add project, settings, collapse), on the right. */
  children: ReactNode;
  /** Floating window: the footer is where it's dragged from. */
  drag: { "data-tauri-drag-region"?: boolean };
}

function Row({
  label,
  w,
  now,
  title,
}: {
  label: string;
  w: UsageWindow | null;
  now: number;
  title?: string;
}) {
  const v = usageNow(w, now);
  if (!w || !v) return null;
  const done = v.reset;
  const pct = Math.round(w.percent);
  return (
    <div className={`usage-row${done ? " stale" : ""}`} title={title}>
      <span className="usage-label">{label}</span>
      <span className="usage-bar">
        <span
          className={`usage-fill ${usageLevel(v.percent)}`}
          style={{ width: `${Math.min(100, v.percent)}%` }}
        />
      </span>
      <span className="usage-pct">{done ? "–" : `${pct}%`}</span>
      <span className="usage-reset">
        {done ? "reset" : w.resets_at !== null ? resetTime(w.resets_at, now) : ""}
      </span>
    </div>
  );
}

/** The foot of the sidebar: subscription usage (the numbers /usage shows) on the left, the
 * sidebar's controls on the right. In a floating window it's the drag handle. */
export function UsageFooter({ usage, now, children, drag }: Props) {
  return (
    <footer
      className="usage"
      {...drag}
      title={
        usage
          ? `Usage limits, updated ${elapsed(usage.updated_at, now)} ago (they update after each turn)`
          : undefined
      }
    >
      <div className="usage-rows" {...drag}>
        {usage ? (
          <>
            <Row label="Session" w={usage.five_hour} now={now} />
            <Row label="Week" w={usage.seven_day} now={now} />
            <Row
              label="Fable"
              w={usage.fable}
              now={now}
              title={
                usage.fable_updated_at !== null
                  ? `Weekly Fable limit, updated ${elapsed(usage.fable_updated_at, now)} ago`
                  : undefined
              }
            />
            {usage.status === "rejected" && (
              <div className="usage-warn">
                {usage.limit_type === "seven_day_overage_included"
                  ? "Fable limit reached"
                  : "Limit reached"}
              </div>
            )}
            {usage.using_overage && <div className="usage-warn">Using extra usage</div>}
          </>
        ) : (
          <span className="muted small">Usage limits: checking…</span>
        )}
      </div>
      <div className="sidebar-controls">{children}</div>
    </footer>
  );
}
